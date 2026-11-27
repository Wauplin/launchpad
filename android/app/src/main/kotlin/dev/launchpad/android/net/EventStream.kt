package dev.launchpad.android.net

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import dev.launchpad.android.data.LpJson
import dev.launchpad.android.data.str
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.job
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.longOrNull
import java.io.IOException

/** One event of Home's bus: `{bus, seq, topic, type, payload}`. */
data class BusEvent(val bus: Long, val seq: Long, val topic: String, val type: String, val payload: JsonElement)

/** The phone's link to Home (design.md "States"). */
sealed interface Conn {
    /** Opening the stream. */
    data object Connecting : Conn
    data object Live : Conn
    /** `lastData`: Unix ms when data last arrived. */
    data class Offline(val lastData: Long?) : Conn
    /** The Space sleeps or builds. */
    data object Starting : Conn
    /** Home answers 503 `retryable`: writes wait. */
    data object Updating : Conn
    /** Hugging Face no longer accepts the login: Sign in again reruns only that step. */
    data object AuthExpired : Conn
    /** "Home is too old" or "Update the app". */
    data class Incompatible(val message: String) : Conn
    /** The app is not on screen: the stream is closed on purpose (push covers the background). */
    data object Paused : Conn
}

/** The Home API level this app speaks, and the capabilities it needs (`GET /api/version`). */
const val CLIENT_API = 1
val NEEDED_CAPABILITIES = listOf("phone_device")

/**
 * The live stream: `GET /api/events` (server-sent events) with the device and Hugging Face headers, as
 * crates/server/src/ws.rs documents it. `hello` offers a position; `replayed` (the missed events came first) or
 * `resync` (reload what is shown) confirms it; each event then advances it, and a reconnect sends it back as
 * `Last-Event-ID`. Open only while the app is visible, plus 30 s. Reconnects at once on a network change, otherwise
 * after 1, 5, 15, 30 s, then every 60 s, and never more often than every 5 s while Home does not answer.
 */
class EventStream(context: Context, private val home: HomeClient, private val scope: CoroutineScope, private val onRevoked: suspend () -> Unit) {
    private val _conn = MutableStateFlow<Conn>(Conn.Paused)
    val conn: StateFlow<Conn> = _conn
    /** Every bus event of the subscribed topics, in order. */
    val events = MutableSharedFlow<BusEvent>(extraBufferCapacity = 1024)
    /** Home could not replay what was missed (or this is the first connection): reload what the screens show. */
    val resync = MutableSharedFlow<Unit>(extraBufferCapacity = 4)
    /** Unix ms of the last message from Home (kept across restarts by the repository). */
    val lastData = MutableStateFlow<Long?>(null)
    /** Home's capabilities, once `/api/version` answered. */
    val capabilities = MutableStateFlow<List<String>>(emptyList())

    private val base = listOf("today", "sessions", "chats", "bots", "channels", "folios", "workspaces", "projects", "proposals", "status")
    private val extra = HashMap<String, Int>()
    private var job: Job? = null
    private var stopper: Job? = null
    private val wake = Channel<Unit>(Channel.CONFLATED)
    @Volatile private var position: Pair<String, Long>? = null
    @Volatile private var current: okhttp3.Call? = null
    @Volatile private var topicsChanged = false
    private var versionChecked = false

    init {
        val manager = context.getSystemService(ConnectivityManager::class.java)
        runCatching {
            manager?.registerDefaultNetworkCallback(object : ConnectivityManager.NetworkCallback() {
                override fun onAvailable(network: Network) { networkChanged() }
            })
        }
    }

    private fun networkChanged() {
        // A new network: the old connection is dead even if it has not noticed yet.
        if (_conn.value != Conn.Live) wake.trySend(Unit)
    }

    private fun topics(): List<String> = synchronized(extra) { base + extra.keys }

    /** Subscribes to one more topic prefix (a shown trace: `session/<id>`) until the returned function runs. */
    fun watch(topic: String): () -> Unit {
        synchronized(extra) { extra[topic] = (extra[topic] ?: 0) + 1 }
        reopen()
        return {
            val gone = synchronized(extra) {
                val n = (extra[topic] ?: 1) - 1
                if (n <= 0) extra.remove(topic) else extra[topic] = n
                n <= 0
            }
            if (gone) reopen()
        }
    }

    private fun reopen() {
        topicsChanged = true
        current?.cancel()
    }

    /** The app came to the screen, or left it (the stream closes 30 s later). */
    fun setVisible(visible: Boolean) {
        stopper?.cancel()
        if (visible) start() else stopper = scope.launch {
            delay(30_000)
            stop()
        }
    }

    fun start() {
        if (job?.isActive == true) return
        if (home.origin.isEmpty()) return
        job = scope.launch { run() }
    }

    fun stop() {
        job?.cancel()
        job = null
        current?.cancel()
        _conn.value = Conn.Paused
    }

    /** Forget the position (another Home, or signed out). */
    fun reset() {
        stop()
        position = null
        versionChecked = false
        lastData.value = null
    }

    /** A screen's own call learned that Hugging Face no longer accepts the login (the stream may not have noticed yet). */
    fun markAuthExpired() {
        if (job?.isActive == true && _conn.value != Conn.AuthExpired) _conn.value = Conn.AuthExpired
    }

    /** The owner tapped Retry now, or signed in again. */
    fun retryNow() {
        if (job?.isActive == true) wake.trySend(Unit) else start()
    }

    private suspend fun run() {
        var attempt = 0
        var lastAttempt = 0L
        _conn.value = Conn.Connecting
        while (scope.isActive && kotlin.coroutines.coroutineContext.isActive) {
            // Never more often than every 5 s while Home does not answer.
            if (attempt > 1) (lastAttempt + 5000 - System.currentTimeMillis()).takeIf { it > 0 }?.let { delay(it) }
            lastAttempt = System.currentTimeMillis()
            var waitForOwner = false
            try {
                if (!versionChecked) checkVersion()
                val wasLive = read()
                attempt = if (wasLive) 0 else attempt
                if (topicsChanged) {
                    topicsChanged = false
                    continue
                }
                // The stream dropped (Home restarts, the network changed): try again after 1 s.
                attempt += 1
                if (attempt >= OFFLINE_AFTER) _conn.value = Conn.Offline(lastData.value) else if (_conn.value == Conn.Live) _conn.value = Conn.Connecting
            } catch (e: HomeException) {
                attempt += 1
                when (e.failure) {
                    Failure.Revoked -> {
                        onRevoked()
                        return
                    }
                    Failure.AuthExpired -> {
                        _conn.value = Conn.AuthExpired
                        waitForOwner = true
                    }
                    Failure.Starting -> _conn.value = Conn.Starting
                    Failure.Updating -> _conn.value = Conn.Updating
                    else -> if (e.status == INCOMPATIBLE) {
                        _conn.value = Conn.Incompatible(e.message ?: "")
                        waitForOwner = true
                    } else if (attempt >= OFFLINE_AFTER || _conn.value is Conn.Offline) _conn.value = Conn.Offline(lastData.value) else _conn.value = Conn.Connecting
                }
            }
            if (waitForOwner) {
                // Sign in again, or an update: only the owner (Retry now) or a new sign-in goes on.
                wake.receive()
                attempt = 0
                versionChecked = false
                continue
            }
            withTimeoutOrNull(backoff(attempt)) { wake.receive() }
        }
    }

    private companion object {
        const val OFFLINE_AFTER = 3
        const val INCOMPATIBLE = -2

        /** 1 s after a stream that dropped, then 5, 15, 30 s and every 60 s. */
        fun backoff(attempt: Int): Long = when (attempt) {
            0, 1 -> 1000L
            2 -> 5000L
            3 -> 15_000L
            4 -> 30_000L
            else -> 60_000L
        }
    }

    private suspend fun checkVersion() {
        val v = try {
            home.getJson("/api/version")
        } catch (e: HomeException) {
            if (e.status == 404) throw HomeException(Failure.Other, "Home is too old: update Home", INCOMPATIBLE) else throw e
        }
        val obj = v as? JsonObject
        val api = (obj?.get("api") as? JsonPrimitive)?.intOrNull
        val minClient = (obj?.get("min_client") as? JsonPrimitive)?.intOrNull ?: 1
        val caps = (obj?.get("capabilities") as? JsonArray)?.mapNotNull { (it as? JsonPrimitive)?.content } ?: emptyList()
        capabilities.value = caps
        if (api == null || NEEDED_CAPABILITIES.any { it !in caps }) throw HomeException(Failure.Other, "Home is too old: update Home", INCOMPATIBLE)
        if (CLIENT_API < minClient) throw HomeException(Failure.Other, "Update the app: Home needs a newer one", INCOMPATIBLE)
        versionChecked = true
    }

    /** Opens the stream and reads it until it ends. True when it was live (so the retry delay starts over). */
    private suspend fun read(): Boolean {
        val subscribed = topics()
        val (call, response) = home.events(position, subscribed)
        current = call
        var live = false
        var offered: Pair<String, Long>? = null
        val onCancel = kotlin.coroutines.coroutineContext.job.invokeOnCompletion { call.cancel() }
        try {
            withContext(Dispatchers.IO) {
                response.use { r ->
                    val source = r.body.source()
                    val data = StringBuilder()
                    while (true) {
                        val line = source.readUtf8Line() ?: break
                        if (line.isEmpty()) {
                            if (data.isNotEmpty()) {
                                val text = data.toString()
                                data.clear()
                                lastData.value = System.currentTimeMillis()
                                val msg = runCatching { LpJson.parseToJsonElement(text) as? JsonObject }.getOrNull() ?: continue
                                when (val type = msg.str("type")) {
                                    "hello" -> {
                                        offered = (msg.str("boot") ?: "") to ((msg["bus"] as? JsonPrimitive)?.longOrNull ?: 0L)
                                        live = true
                                        _conn.value = Conn.Live
                                    }
                                    "replayed", "resync" -> {
                                        offered?.let { position = it }
                                        offered = null
                                        if (type == "resync") resync.tryEmit(Unit)
                                    }
                                    "ping", "pong" -> {}
                                    else -> {
                                        val topic = msg.str("topic") ?: continue
                                        val bus = (msg["bus"] as? JsonPrimitive)?.longOrNull ?: 0L
                                        // Only events (never control messages) advance the position.
                                        position?.let { position = it.first to bus }
                                        // An older Home ignores `topics` and sends everything.
                                        if (subscribed.none { topic == it || topic.startsWith(it) }) continue
                                        events.emit(BusEvent(bus, (msg["seq"] as? JsonPrimitive)?.longOrNull ?: 0L, topic, type ?: "", msg["payload"] ?: JsonNull))
                                    }
                                }
                            }
                        } else if (line.startsWith("data:")) {
                            if (data.isNotEmpty()) data.append('\n')
                            data.append(line.removePrefix("data:").removePrefix(" "))
                        } else if (line.startsWith(":")) {
                            // `: keepalive` every 15 s.
                            lastData.value = System.currentTimeMillis()
                        }
                    }
                }
            }
        } catch (_: IOException) {
            // Dropped, timed out (no keepalive for 45 s) or cancelled for new topics: the caller reconnects.
        } finally {
            onCancel.dispose()
            current = null
        }
        return live
    }
}
