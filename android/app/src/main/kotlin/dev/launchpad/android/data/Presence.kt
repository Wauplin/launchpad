package dev.launchpad.android.data

import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.ProcessLifecycleOwner
import dev.launchpad.android.Graph
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

/**
 * What the phone shows, for Home's "seen" and notification rules (live.md "Seen, presence and sends"): the screen
 * reports the sessions it shows ([show]), touches count as input ([input]), and the app posts
 * `POST /api/devices/self/presence` `{focused, sessions, idle_ms}` on every change, on input (at most every 5 s) and
 * every 25 s while visible; `{focused: false, sessions: []}` when it leaves the screen. Only when Home has the
 * capability `device_presence`.
 */
object Presence {
    private var sessions: List<String> = emptyList()
    private var visible = false
    private var lastInput = 0L
    private var lastSent = 0L
    private var loop: Job? = null
    private var started = false

    fun start() {
        if (started) return
        started = true
        ProcessLifecycleOwner.get().lifecycle.addObserver(object : DefaultLifecycleObserver {
            override fun onStart(owner: LifecycleOwner) { visible = true; lastInput = System.currentTimeMillis(); post(); keepAlive() }
            override fun onStop(owner: LifecycleOwner) { visible = false; loop?.cancel(); post() }
        })
    }

    /** The sessions the current screen shows (an agent, a bot chat's session); empty for lists. */
    fun show(ids: List<String>) {
        if (ids == sessions) return
        sessions = ids
        post()
    }

    /** A touch or a key: the owner uses the phone now. */
    fun input() {
        val now = System.currentTimeMillis()
        lastInput = now
        if (now - lastSent > 5_000) post()
    }

    /** Whether the owner used the phone in the last minute (the web's rule for "seen"). */
    val active: Boolean get() = visible && System.currentTimeMillis() - lastInput < 60_000

    private fun keepAlive() {
        loop?.cancel()
        loop = Graph.scope.launch {
            while (true) {
                delay(25_000)
                if (visible) post()
            }
        }
    }

    private fun post() {
        if (!Graph.linked.value || "device_presence" !in Graph.stream.capabilities.value) return
        lastSent = System.currentTimeMillis()
        val body = buildJsonObject {
            put("focused", visible)
            put("sessions", buildJsonArray { if (visible) sessions.forEach { add(JsonPrimitive(it)) } })
            put("idle_ms", (System.currentTimeMillis() - lastInput).coerceAtLeast(0))
        }
        Graph.scope.launch { runCatching { Graph.home.write("POST", "/api/devices/self/presence", body) } }
    }
}
