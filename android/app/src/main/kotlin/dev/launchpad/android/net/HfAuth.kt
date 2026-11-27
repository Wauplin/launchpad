package dev.launchpad.android.net

import android.net.Uri
import android.util.Base64
import dev.launchpad.android.BuildConfig
import dev.launchpad.android.data.LpJson
import dev.launchpad.android.data.Secrets
import dev.launchpad.android.data.Store
import dev.launchpad.android.data.bool
import dev.launchpad.android.data.str
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.json.JsonElement
import okhttp3.FormBody
import okhttp3.OkHttpClient
import okhttp3.Request
import java.io.IOException
import java.net.InetAddress
import java.net.ServerSocket
import java.net.SocketTimeoutException
import java.security.MessageDigest
import java.security.SecureRandom

/** The Hub could not be asked (no network, a 5xx or 429): not a refusal, so the app retries instead of asking to sign in again. */
class HfTransient(message: String) : IOException("Hugging Face does not answer: $message")

/** Hugging Face refused (the owner cancelled, the refresh token is no longer accepted). */
class HfRefused(message: String) : Exception(message)

/** A Launchpad Space found on the account or one of its organizations. */
data class FoundHome(val space: String, val origin: String, val stage: String?, val answers: Boolean, val own: Boolean)

sealed interface HomeStatus {
    data class Home(val complete: Boolean) : HomeStatus
    data object Starting : HomeStatus
    data class Unreachable(val why: String) : HomeStatus
}

/**
 * Sign in with Hugging Face as a native app, the desktop companion's rules (desktop/src/oauth.rs, hf.rs): authorization
 * code with PKCE (S256), the shared public client, and the loopback redirect `http://127.0.0.1:<port>/callback`
 * (RFC 8252 §7.3; Hugging Face accepts any port for the registered `http://127.0.0.1/callback`). The tokens live in
 * the encrypted store and go only to huggingface.co and the Space's own `*.hf.space` address.
 */
class HfAuth(private val http: OkHttpClient, private val store: Store) {
    private val mutex = Mutex()

    val clientId: String get() = BuildConfig.HF_CLIENT_ID

    /** A sign-in in progress: the page to open in a Custom Tab, then [await] the redirect. */
    class Pending internal constructor(val authorizeUrl: Uri, internal val server: ServerSocket, internal val redirect: String, internal val state: String, internal val verifier: String) {
        fun cancel() { runCatching { server.close() } }
    }

    /** Starts listening on a free loopback port (only while signing in) and builds the authorize URL. */
    fun begin(): Pending {
        val server = ServerSocket(0, 4, InetAddress.getByName("127.0.0.1"))
        val redirect = "http://127.0.0.1:${server.localPort}$CALLBACK"
        val (verifier, state) = random(48) to random(16)
        val url = Uri.parse("$HUB/oauth/authorize").buildUpon()
            .appendQueryParameter("client_id", clientId)
            .appendQueryParameter("redirect_uri", redirect)
            .appendQueryParameter("response_type", "code")
            .appendQueryParameter("scope", SCOPES)
            .appendQueryParameter("state", state)
            .appendQueryParameter("code_challenge", challenge(verifier))
            .appendQueryParameter("code_challenge_method", "S256")
            .build()
        return Pending(url, server, redirect, state, verifier)
    }

    /**
     * Waits (up to 5 minutes) for the browser's redirect with our `state`, answers it with a redirect back to the app
     * (`dev.launchpad.android://signed-in`, which carries nothing), exchanges the code and stores the tokens.
     * `onRedirect` runs as soon as the code arrived, so the app can come back to the front itself.
     */
    suspend fun await(pending: Pending, onRedirect: () -> Unit) {
        val code = try {
            withContext(Dispatchers.IO) { listen(pending) }
        } finally {
            pending.cancel()
        }
        onRedirect()
        val answer = token(mapOf("grant_type" to "authorization_code", "code" to code, "redirect_uri" to pending.redirect, "client_id" to clientId, "code_verifier" to pending.verifier))
        mutex.withLock { keep(answer, store.secrets.value) }
    }

    private suspend fun listen(pending: Pending): String {
        val deadline = System.currentTimeMillis() + SIGN_IN_TIMEOUT_MS
        pending.server.soTimeout = 1000
        while (true) {
            kotlin.coroutines.coroutineContext.ensureActive()
            if (System.currentTimeMillis() > deadline) throw HfRefused("The sign-in was not approved within 5 minutes")
            val socket = try {
                pending.server.accept()
            } catch (_: SocketTimeoutException) {
                continue
            } catch (e: IOException) {
                throw HfRefused("The sign-in was cancelled")
            }
            socket.use { s ->
                s.soTimeout = 10_000
                val line = runCatching { s.getInputStream().bufferedReader().readLine() }.getOrNull().orEmpty()
                val result = parseCallback(line, pending.state)
                val (status, text) = when (result) {
                    is Callback.Code -> "302 Found" to "Launchpad is signed in to Hugging Face."
                    is Callback.Refused -> "302 Found" to "Hugging Face did not sign Launchpad in."
                    Callback.Other -> "404 Not Found" to "Not found"
                }
                val link = if (result == Callback.Other) "" else "<p style=\"font:15px sans-serif;margin:0 3em\"><a href=\"$RETURN_URL\">Return to Launchpad</a></p>"
                val page = "<!doctype html><meta charset=utf-8><meta name=viewport content=\"width=device-width\"><title>Launchpad</title><p style=\"font:15px sans-serif;margin:3em\">$text</p>$link"
                val location = if (result == Callback.Other) "" else "Location: $RETURN_URL\r\n"
                val bytes = page.toByteArray()
                runCatching {
                    s.getOutputStream().apply {
                        write("HTTP/1.1 $status\r\n${location}Content-Type: text/html; charset=utf-8\r\nContent-Length: ${bytes.size}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n".toByteArray())
                        write(bytes)
                        flush()
                    }
                }
                when (result) {
                    is Callback.Code -> return result.code
                    is Callback.Refused -> throw HfRefused("Hugging Face refused: ${result.why}")
                    Callback.Other -> {}
                }
            }
        }
    }

    private sealed interface Callback {
        data class Code(val code: String) : Callback
        data class Refused(val why: String) : Callback
        data object Other : Callback
    }

    /** Reads the request line of the browser's redirect (`GET /callback?code=…&state=… HTTP/1.1`). */
    private fun parseCallback(requestLine: String, state: String): Callback {
        val parts = requestLine.trim().split(Regex("\\s+"))
        if (parts.size < 2 || parts[0] != "GET") return Callback.Other
        val url = runCatching { Uri.parse("http://127.0.0.1${parts[1]}") }.getOrNull() ?: return Callback.Other
        if (url.path != CALLBACK || url.getQueryParameter("state") != state) return Callback.Other
        url.getQueryParameter("error")?.let { return Callback.Refused(url.getQueryParameter("error_description") ?: it) }
        val code = url.getQueryParameter("code")
        return if (code.isNullOrEmpty()) Callback.Refused("Hugging Face returned no authorization code") else Callback.Code(code)
    }

    @Serializable
    private data class TokenAnswer(@SerialName("access_token") val accessToken: String = "", @SerialName("expires_in") val expiresIn: Long? = null, @SerialName("refresh_token") val refreshToken: String? = null)

    private suspend fun token(form: Map<String, String>): TokenAnswer = withContext(Dispatchers.IO) {
        val body = FormBody.Builder().apply { form.forEach { (k, v) -> add(k, v) } }.build()
        val response = try {
            http.newCall(Request.Builder().url("$HUB/oauth/token").post(body).build()).execute()
        } catch (e: IOException) {
            throw HfTransient(e.message ?: "no connection")
        }
        response.use { r ->
            if (r.code >= 500 || r.code == 429) throw HfTransient("HTTP ${r.code}")
            val json = runCatching { LpJson.parseToJsonElement(r.body.string()) }.getOrNull()
            if (!r.isSuccessful || json.str("access_token").isNullOrEmpty()) {
                throw HfRefused("Hugging Face refused the sign-in: ${json.str("error_description") ?: json.str("error") ?: "HTTP ${r.code}"}")
            }
            LpJson.decodeFromJsonElement(TokenAnswer.serializer(), json!!)
        }
    }

    private suspend fun keep(answer: TokenAnswer, current: Secrets): String {
        store.setSecrets(
            current.copy(
                hfAccess = answer.accessToken,
                hfExpiresAt = System.currentTimeMillis() + (answer.expiresIn ?: 3600) * 1000,
                hfRefresh = answer.refreshToken?.takeIf { it.isNotEmpty() } ?: current.hfRefresh,
            ),
        )
        return answer.accessToken
    }

    val signedIn: Boolean get() = store.secrets.value.hfRefresh.isNotEmpty() || store.secrets.value.hfAccess.isNotEmpty()

    /** A valid bearer for the Hub and the Space, refreshed when it expires soon. */
    suspend fun bearer(): String = mutex.withLock {
        val s = store.secrets.value
        if (s.hfAccess.isNotEmpty() && s.hfExpiresAt > System.currentTimeMillis() + MARGIN_MS) return s.hfAccess
        if (s.hfRefresh.isEmpty()) throw HfRefused("Sign in with Hugging Face again")
        keep(token(mapOf("grant_type" to "refresh_token", "refresh_token" to s.hfRefresh, "client_id" to clientId)), s)
    }

    /** Drops the cached access token so the next [bearer] refreshes (after a 401 of the Space). */
    suspend fun expire() = mutex.withLock {
        store.setSecrets(store.secrets.value.copy(hfAccess = "", hfExpiresAt = 0))
    }

    private suspend fun hub(path: String): JsonElement = withContext(Dispatchers.IO) {
        val bearer = bearer()
        val response = try {
            http.newCall(Request.Builder().url("$HUB$path").header("Authorization", "Bearer $bearer").build()).execute()
        } catch (e: IOException) {
            throw HfTransient(e.message ?: "no connection")
        }
        response.use { r ->
            if (!r.isSuccessful) throw IOException("Hugging Face answered ${r.code} for ${path.substringBefore('?')}")
            LpJson.parseToJsonElement(r.body.string())
        }
    }

    @Serializable private data class Org(val name: String = "")
    @Serializable private data class Whoami(val name: String = "", val orgs: List<Org> = emptyList())
    @Serializable private data class CardData(val title: String? = null)
    @Serializable private data class Runtime(val stage: String? = null)
    @Serializable private data class SpaceInfo(val id: String = "", val host: String? = null, val subdomain: String? = null, val runtime: Runtime? = null, val cardData: CardData? = null, val tags: List<String> = emptyList())

    /** The account's name, then the Launchpad Spaces of the account and of its organizations (tag `launchpad-home`, or the title of Spaces deployed before the tag). */
    suspend fun discover(): Pair<String, List<FoundHome>> {
        val me = LpJson.decodeFromJsonElement(Whoami.serializer(), hub("/api/whoami-v2"))
        val found = mutableListOf<FoundHome>()
        for (author in listOf(me.name) + me.orgs.map { it.name }) {
            fun q(vararg pairs: Pair<String, String>) = Uri.Builder().apply { pairs.forEach { (k, v) -> appendQueryParameter(k, v) } }.build().encodedQuery
            val list = ListSerializer(SpaceInfo.serializer())
            val tagged = runCatching { LpJson.decodeFromJsonElement(list, hub("/api/spaces?${q("author" to author, "filter" to SPACE_TAG)}")) }.getOrElse { if (it is HfTransient || it is HfRefused) throw it else emptyList() }
            val named = runCatching { LpJson.decodeFromJsonElement(list, hub("/api/spaces?${q("author" to author, "search" to "launchpad", "full" to "true")}")) }.getOrElse { emptyList() }
            val ids = tagged.map { it.id }.toMutableList()
            for (s in named) if ((s.cardData?.title == SPACE_TITLE || SPACE_TAG in s.tags) && s.id !in ids) ids += s.id
            for (id in ids) {
                val info = runCatching { LpJson.decodeFromJsonElement(SpaceInfo.serializer(), hub("/api/spaces/$id")) }.getOrNull() ?: continue
                val origin = (info.host ?: info.subdomain?.let { "https://$it.hf.space" })?.takeIf(::spaceOrigin) ?: continue
                val stage = info.runtime?.stage
                // Only the account's own running Spaces are asked: a request wakes a sleeping one, and an organization's
                // Space runs code other members may change.
                val own = author == me.name
                val answers = own && stage == "RUNNING" && homeStatus(origin) is HomeStatus.Home
                found += FoundHome(id, origin, stage, answers, own)
            }
        }
        return me.name to found
    }

    /** `GET /_home/status` of a Space, as the phone client. */
    suspend fun homeStatus(origin: String): HomeStatus = withContext(Dispatchers.IO) {
        if (!spaceOrigin(origin)) return@withContext HomeStatus.Unreachable("Not a Hugging Face Space address")
        val bearer = runCatching { bearer() }.getOrElse { return@withContext HomeStatus.Unreachable("Sign in with Hugging Face again") }
        try {
            http.newCall(Request.Builder().url("$origin/_home/status").header("Authorization", "Bearer $bearer").header(CLIENT_HEADER, clientName()).build()).execute().use { r ->
                val json = runCatching { LpJson.parseToJsonElement(r.body.string()) }.getOrNull()
                when {
                    r.code == 200 && json.bool("launchpad") == true -> HomeStatus.Home(json.bool("complete") ?: false)
                    r.code in listOf(401, 403, 404) -> HomeStatus.Unreachable("Hugging Face refused access to this Space")
                    else -> HomeStatus.Starting
                }
            }
        } catch (e: IOException) {
            HomeStatus.Unreachable(e.message ?: "no connection")
        }
    }

    companion object {
        const val HUB = "https://huggingface.co"
        const val SCOPES = "openid profile read-repos"
        const val CALLBACK = "/callback"
        const val SPACE_TAG = "launchpad-home"
        const val SPACE_TITLE = "Launchpad Home"
        /** Where the loopback answer sends the browser: MainActivity's intent filter. It carries no secret. */
        const val RETURN_URL = "dev.launchpad.android://signed-in"
        /** Renew this long before the access token expires. */
        private const val MARGIN_MS = 5 * 60 * 1000L
        private const val SIGN_IN_TIMEOUT_MS = 5 * 60 * 1000L

        private fun b64(bytes: ByteArray) = Base64.encodeToString(bytes, Base64.URL_SAFE or Base64.NO_PADDING or Base64.NO_WRAP)
        private fun random(bytes: Int) = b64(ByteArray(bytes).also { SecureRandom().nextBytes(it) })
        /** The S256 challenge of a PKCE verifier (RFC 7636 §4.2). */
        private fun challenge(verifier: String) = b64(MessageDigest.getInstance("SHA-256").digest(verifier.toByteArray()))

        /** A Space's own address: the only place besides the Hub that ever receives the Hugging Face token. */
        fun spaceOrigin(origin: String): Boolean {
            if (!origin.startsWith("https://") || !origin.endsWith(".hf.space")) return false
            val sub = origin.substring("https://".length, origin.length - ".hf.space".length)
            return sub.isNotEmpty() && sub.all { it in 'a'..'z' || it in '0'..'9' || it == '-' }
        }
    }
}
