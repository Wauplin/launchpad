package dev.launchpad.android.net

import dev.launchpad.android.BuildConfig
import dev.launchpad.android.data.LpJson
import dev.launchpad.android.data.Store
import dev.launchpad.android.data.str
import dev.launchpad.android.gen.Actions
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.withContext
import kotlinx.serialization.KSerializer
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import okhttp3.Call
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import okhttp3.Response
import java.io.IOException
import java.net.ConnectException
import java.net.NoRouteToHostException
import java.net.URLEncoder
import java.net.UnknownHostException
import java.util.concurrent.TimeUnit

const val CLIENT_HEADER = "X-Launchpad-Client"
const val DEVICE_HEADER = "X-Launchpad-Device"

/** How the app names itself to the gateway and Home: `android/<version>`, and never an Origin. */
fun clientName() = "android/${BuildConfig.VERSION_NAME}"

/** Why a request to Home failed (desktop/src/conn.rs). */
enum class Failure { Network, Revoked, AuthExpired, Starting, Updating, Other }

/**
 * A failed call. `uncertain`: a write whose answer never came (it may have reached Home): show it as uncertain and
 * never repeat it automatically. `sent` false: the request never left the phone.
 */
class HomeException(val failure: Failure, message: String, val status: Int = 0, val uncertain: Boolean = false, val sent: Boolean = true) : IOException(message)

/** A raw answer, for the folio reader's bridge and downloads. */
class RawResponse(val status: Int, val contentType: String?, val bytes: ByteArray)

/**
 * Reads a failed answer of a Space's gateway or Home. Home's own refusal of a device says "pair this browser" (the
 * daemon) or "Connect this … first" (the gateway, no device header): only those mean Revoked, which deletes the
 * secrets. The Hub's own page (not JSON) on 401/403 means the Hugging Face login is no longer accepted.
 */
fun classify(status: Int, body: String): Failure {
    val json = body.trimStart().startsWith("{")
    val lower = body.lowercase()
    return when {
        status == 401 && json && ("pair this browser" in lower || "connect this" in lower) -> Failure.Revoked
        (status == 401 || status == 403) && !json -> Failure.AuthExpired
        status == 503 && json && "retryable" in lower -> Failure.Updating
        status in 502..504 && !json -> Failure.Starting
        status in 502..504 -> Failure.Network
        !json && status != 404 -> Failure.Starting
        else -> Failure.Other
    }
}

/**
 * The Home client. Every call names the app (`X-Launchpad-Client: android/<version>`), sends this phone's device secret
 * (`X-Launchpad-Device`) and the owner's Hugging Face token (the Hub's private-Space check; the gateway strips it
 * before Home). Never an Origin, never a cookie, no redirects followed (a redirect elsewhere would carry the device
 * secret out), and writes are never retried by the HTTP library.
 */
class HomeClient(base: OkHttpClient, private val store: Store, private val hf: HfAuth) {
    private val http = base.newBuilder().followRedirects(false).followSslRedirects(false).retryOnConnectionFailure(false).build()
    private val streaming = http.newBuilder().readTimeout(45, TimeUnit.SECONDS).build()

    /** Failures every screen cares about: Revoked (delete everything), AuthExpired (Sign in again), Updating. */
    val failures = MutableSharedFlow<Failure>(extraBufferCapacity = 8)

    val origin: String get() = store.link.value.origin

    private suspend fun builder(path: String): Request.Builder {
        val origin = store.link.value.origin
        // The Hugging Face token goes only to a Space's own address.
        if (!HfAuth.spaceOrigin(origin)) throw HomeException(Failure.Other, "This phone is not connected to a Home", sent = false)
        val bearer = try {
            hf.bearer()
        } catch (e: HfTransient) {
            throw HomeException(Failure.Network, e.message ?: "Hugging Face does not answer", sent = false)
        } catch (e: HfRefused) {
            failures.tryEmit(Failure.AuthExpired)
            throw HomeException(Failure.AuthExpired, e.message ?: "Sign in again", sent = false)
        }
        return Request.Builder().url(origin + path)
            .header("Authorization", "Bearer $bearer")
            .header(DEVICE_HEADER, store.secrets.value.device)
            .header(CLIENT_HEADER, clientName())
            .header("User-Agent", "launchpad-android/${BuildConfig.VERSION_NAME}")
    }

    private fun fail(status: Int, text: String): HomeException {
        val failure = classify(status, text)
        val message = runCatching { LpJson.parseToJsonElement(text) }.getOrNull().str("error") ?: when (failure) {
            Failure.Starting -> "Home is starting"
            Failure.AuthExpired -> "Sign in again"
            else -> "Home answered $status"
        }
        if (failure == Failure.Revoked || failure == Failure.AuthExpired || failure == Failure.Updating) failures.tryEmit(failure)
        return HomeException(failure, message, status)
    }

    /** One request; `write` decides how an I/O failure is read (uncertain or not sent). */
    private suspend fun send(method: String, path: String, body: ByteArray?, contentType: String?, headers: Map<String, String>, write: Boolean): Response = withContext(Dispatchers.IO) {
        var refreshed = false
        while (true) {
            val request = builder(path).apply {
                headers.forEach { (k, v) -> header(k, v) }
                method(method, body?.toRequestBody((contentType ?: JSON).toMediaType()) ?: if (method == "GET" || method == "HEAD") null else ByteArray(0).toRequestBody(null))
            }.build()
            val response = try {
                http.newCall(request).execute()
            } catch (e: IOException) {
                val unsent = e is ConnectException || e is UnknownHostException || e is NoRouteToHostException || e is javax.net.ssl.SSLHandshakeException
                throw HomeException(Failure.Network, if (unsent) "No connection to Home" else "Home did not answer in time", uncertain = write && !unsent, sent = !unsent)
            }
            if (response.code < 400) return@withContext response
            val text = runCatching { response.body.string() }.getOrDefault("")
            response.close()
            val failure = classify(response.code, text)
            // A Space's 401 may be an expired access token: refresh once.
            if (failure == Failure.AuthExpired && !refreshed) {
                hf.expire()
                refreshed = true
                continue
            }
            throw fail(response.code, text)
        }
        @Suppress("UNREACHABLE_CODE")
        error("unreachable")
    }

    suspend fun getJson(path: String): JsonElement = send("GET", path, null, null, emptyMap(), false).use { r ->
        runCatching { LpJson.parseToJsonElement(r.body.string()) }.getOrElse { throw HomeException(Failure.Other, "Unexpected answer from Home") }
    }

    suspend fun <T> get(path: String, serializer: KSerializer<T>): T = send("GET", path, null, null, emptyMap(), false).use { r ->
        runCatching { LpJson.decodeFromString(serializer, r.body.string()) }.getOrElse { throw HomeException(Failure.Other, "Unexpected answer from Home: ${it.message?.take(120)}") }
    }

    /**
     * A JSON write. Home is updating (503 `retryable`): Home refused it, so it waits and sends again once Home is
     * back (up to 10 minutes). No answer in time: [HomeException.uncertain], never repeated here. `key` becomes the
     * `Idempotency-Key`, so a send the owner repeats by hand is done once on Home.
     */
    suspend fun write(method: String, path: String, body: JsonElement? = null, key: String? = null): JsonElement {
        val bytes = (body ?: JsonObject(emptyMap())).let { LpJson.encodeToString(JsonElement.serializer(), it).toByteArray() }
        val headers = if (key != null) mapOf("Idempotency-Key" to key) else emptyMap()
        val deadline = System.currentTimeMillis() + 10 * 60 * 1000
        while (true) {
            try {
                return send(method, path, if (method == "DELETE" && body == null) null else bytes, JSON, headers, true).use { r ->
                    runCatching { LpJson.parseToJsonElement(r.body.string()) }.getOrDefault(JsonNull)
                }
            } catch (e: HomeException) {
                if (e.failure != Failure.Updating || System.currentTimeMillis() > deadline) throw e
                delay(5000)
            }
        }
    }

    /**
     * Runs an action of the action list (`Actions.get(name)`): `params` fill the route's `{param}` segments, `input`
     * is the body (the action's fixed fields are merged in).
     */
    suspend fun runAction(name: String, params: Map<String, String> = emptyMap(), input: JsonObject? = null, key: String? = null): JsonElement {
        val action = Actions.get(name) ?: throw HomeException(Failure.Other, "Unknown action $name", sent = false)
        var route = action.route
        params.forEach { (k, v) -> route = route.replace("{$k}", URLEncoder.encode(v, "UTF-8").replace("+", "%20")) }
        val fixed = action.fixed?.let { runCatching { LpJson.parseToJsonElement(it) as? JsonObject }.getOrNull() }
        val body = if (input == null && fixed == null) null else buildJsonObject {
            input?.forEach { (k, v) -> put(k, v) }
            fixed?.forEach { (k, v) -> put(k, v) }
        }
        return if (action.method == "GET") getJson(route) else write(action.method, route, body, key)
    }

    /** Any call with raw bytes both ways (the folio reader's bridge, uploads, images). */
    suspend fun raw(method: String, path: String, body: ByteArray? = null, contentType: String? = null, headers: Map<String, String> = emptyMap()): RawResponse {
        return try {
            send(method, path, body, contentType, headers, method != "GET").use { r -> RawResponse(r.code, r.header("Content-Type"), r.body.bytes()) }
        } catch (e: HomeException) {
            if (e.status == 0) throw e
            RawResponse(e.status, JSON, LpJson.encodeToString(JsonElement.serializer(), buildJsonObject { put("error", JsonPrimitive(e.message)) }).toByteArray())
        }
    }

    /** `GET /api/events` as a stream (`Last-Event-ID: <boot>:<bus>`, `?topics=` prefixes). The caller reads and closes it. */
    suspend fun events(position: Pair<String, Long>?, topics: Collection<String>): Pair<Call, Response> = withContext(Dispatchers.IO) {
        val query = if (topics.isEmpty()) "" else "?topics=" + topics.joinToString(",") { URLEncoder.encode(it, "UTF-8") }
        var refreshed = false
        while (true) {
            val request = builder("/api/events$query").header("Accept", "text/event-stream").apply {
                position?.let { header("Last-Event-ID", "${it.first}:${it.second}") }
            }.build()
            val call = streaming.newCall(request)
            val response = try {
                call.execute()
            } catch (e: IOException) {
                throw HomeException(Failure.Network, "No connection to Home", sent = false)
            }
            if (response.code < 400) return@withContext call to response
            val text = runCatching { response.body.string() }.getOrDefault("")
            response.close()
            if (classify(response.code, text) == Failure.AuthExpired && !refreshed) {
                hf.expire()
                refreshed = true
                continue
            }
            throw fail(response.code, text)
        }
        @Suppress("UNREACHABLE_CODE")
        error("unreachable")
    }

    /** `POST <space>/_home/phone/connect` with the Hugging Face bearer and no device: Home creates this phone's device. */
    suspend fun connectPhone(origin: String, name: String): JsonElement = withContext(Dispatchers.IO) {
        if (!HfAuth.spaceOrigin(origin)) throw HomeException(Failure.Other, "Not a Hugging Face Space address", sent = false)
        val bearer = try { hf.bearer() } catch (e: HfTransient) { throw HomeException(Failure.Network, e.message ?: "", sent = false) } catch (e: HfRefused) { throw HomeException(Failure.AuthExpired, e.message ?: "", sent = false) }
        val body = LpJson.encodeToString(JsonElement.serializer(), buildJsonObject { put("name", JsonPrimitive(name)) }).toByteArray().toRequestBody(JSON.toMediaType())
        val request = Request.Builder().url("$origin/_home/phone/connect").post(body).header("Authorization", "Bearer $bearer").header(CLIENT_HEADER, clientName()).build()
        try {
            http.newCall(request).execute().use { r ->
                val text = r.body.string()
                val json = runCatching { LpJson.parseToJsonElement(text) }.getOrNull()
                if (r.code >= 400 || json.str("device").isNullOrEmpty()) {
                    val message = json.str("error") ?: when {
                        r.code == 404 || r.code == 403 -> "This Home is too old for the Android app: update Home"
                        classify(r.code, text) == Failure.Starting -> "Home is starting. Try again in a minute."
                        classify(r.code, text) == Failure.AuthExpired -> "Hugging Face refused access to this Space"
                        else -> "Home answered ${r.code}"
                    }
                    throw HomeException(classify(r.code, text), message, r.code)
                }
                json!!
            }
        } catch (e: HomeException) {
            throw e
        } catch (e: IOException) {
            throw HomeException(Failure.Network, "Home does not answer", uncertain = true)
        }
    }

    private companion object {
        const val JSON = "application/json"
    }
}
