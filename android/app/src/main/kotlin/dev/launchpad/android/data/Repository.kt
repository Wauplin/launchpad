package dev.launchpad.android.data

import dev.launchpad.android.gen.Bot
import dev.launchpad.android.gen.BotThread
import dev.launchpad.android.gen.Channel
import dev.launchpad.android.gen.Folio
import dev.launchpad.android.gen.Project
import dev.launchpad.android.gen.Session
import dev.launchpad.android.gen.Today
import dev.launchpad.android.gen.Workspace
import dev.launchpad.android.net.BusEvent
import dev.launchpad.android.net.EventStream
import dev.launchpad.android.net.HomeClient
import dev.launchpad.android.net.HomeException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import kotlinx.serialization.KSerializer
import kotlinx.serialization.Serializable
import kotlinx.serialization.builtins.ListSerializer
import java.net.URLEncoder

@Serializable
private data class ChatThreads(val threads: List<BotThread> = emptyList(), val archived_total: Double = 0.0)

/**
 * Home's records as the screens read them: drawn from the JSON cache at once, refreshed from Home, then kept live
 * from the bus. Today's items and their order are Home's (`today::compute`): nothing here counts or reorders them.
 * One workspace is shown at a time; `threads` and `folios` are that workspace's.
 *
 * To add a list: a `MutableStateFlow`, one `fetch` line in [refreshKey], its key in [ALL] and the topic in [onEvent].
 */
class Repository(private val home: HomeClient, private val stream: EventStream, private val cache: Cache, private val store: Store, private val scope: CoroutineScope) {
    private val _workspaces = MutableStateFlow<List<Workspace>>(emptyList())
    private val _projects = MutableStateFlow<List<Project>>(emptyList())
    private val _sessions = MutableStateFlow<List<Session>>(emptyList())
    private val _bots = MutableStateFlow<List<Bot>>(emptyList())
    private val _channels = MutableStateFlow<List<Channel>>(emptyList())
    private val _today = MutableStateFlow(Today())
    private val _threads = MutableStateFlow<List<BotThread>>(emptyList())
    private val _folios = MutableStateFlow<List<Folio>>(emptyList())
    private val _workspaceId = MutableStateFlow("")
    private val _refreshing = MutableStateFlow(false)
    private val _loaded = MutableStateFlow(false)

    val workspaces: StateFlow<List<Workspace>> = _workspaces
    val projects: StateFlow<List<Project>> = _projects
    /** Light session records (no trace), every workspace, with Home's `group`. */
    val sessions: StateFlow<List<Session>> = _sessions
    val bots: StateFlow<List<Bot>> = _bots
    val channels: StateFlow<List<Channel>> = _channels
    /** Every workspace's waiting items; `Derived.todayOf` narrows them like Home does. */
    val today: StateFlow<Today> = _today
    val threads: StateFlow<List<BotThread>> = _threads
    val folios: StateFlow<List<Folio>> = _folios
    /** The workspace the app shows (remembered on this phone). */
    val workspaceId: StateFlow<String> = _workspaceId
    val refreshing: StateFlow<Boolean> = _refreshing
    /** Something was read, from the cache or from Home. */
    val loaded: StateFlow<Boolean> = _loaded

    private var jobs: List<Job> = emptyList()
    private val pending = LinkedHashSet<String>()
    private var flush: Job? = null

    /** After linking, or at start when linked: the cache first, then Home, then the bus. */
    fun start() {
        if (jobs.isNotEmpty()) return
        _workspaceId.value = store.settings.value.workspace
        readCache()
        jobs = listOf(
            scope.launch { stream.events.collect(::onEvent) },
            scope.launch { stream.resync.collect { refresh() } },
            scope.launch { refresh() },
        )
    }

    private fun readCache() {
        fun <T> list(key: String, s: KSerializer<T>, into: MutableStateFlow<List<T>>) { cache.read(key, ListSerializer(s))?.let { into.value = it; _loaded.value = true } }
        list("workspaces", Workspace.serializer(), _workspaces)
        list("projects", Project.serializer(), _projects)
        list("sessions", Session.serializer(), _sessions)
        list("bots", Bot.serializer(), _bots)
        list("channels", Channel.serializer(), _channels)
        cache.read("today", Today.serializer())?.let { _today.value = it }
        val ws = _workspaceId.value
        if (ws.isNotEmpty()) {
            list("threads-$ws", BotThread.serializer(), _threads)
            list("folios-$ws", Folio.serializer(), _folios)
        }
    }

    private suspend fun <T> fetch(key: String, path: String, serializer: KSerializer<T>, into: MutableStateFlow<T>) {
        try {
            val value = home.get(path, serializer)
            into.value = value
            _loaded.value = true
            cache.write(key, serializer, value)
        } catch (_: HomeException) {
            // The connection state says why; the cached list stays readable.
        }
    }

    private suspend fun refreshKey(key: String) {
        val ws = _workspaceId.value
        val q = URLEncoder.encode(ws, "UTF-8")
        when (key) {
            "workspaces" -> {
                fetch("workspaces", "/api/workspaces", ListSerializer(Workspace.serializer()), _workspaces)
                pickWorkspace()
            }
            "projects" -> fetch("projects", "/api/projects", ListSerializer(Project.serializer()), _projects)
            "sessions" -> fetch("sessions", "/api/sessions", ListSerializer(Session.serializer()), _sessions)
            "bots" -> fetch("bots", "/api/bots", ListSerializer(Bot.serializer()), _bots)
            "channels" -> fetch("channels", "/api/channels", ListSerializer(Channel.serializer()), _channels)
            "today" -> fetch("today", "/api/today", Today.serializer(), _today)
            "threads" -> if (ws.isNotEmpty()) {
                try {
                    val value = home.get("/api/chat-threads?workspace=$q&archived=0", ChatThreads.serializer()).threads
                    if (ws == _workspaceId.value) _threads.value = value
                    cache.write("threads-$ws", ListSerializer(BotThread.serializer()), value)
                } catch (_: HomeException) {}
            }
            "folios" -> if (ws.isNotEmpty()) {
                try {
                    val value = home.get("/api/folios/$q", ListSerializer(Folio.serializer()))
                    if (ws == _workspaceId.value) _folios.value = value
                    cache.write("folios-$ws", ListSerializer(Folio.serializer()), value)
                } catch (_: HomeException) {}
            }
        }
    }

    /** The remembered workspace when it still exists, else the first one. */
    private suspend fun pickWorkspace() {
        val all = _workspaces.value
        if (all.isEmpty() || all.any { it.id == _workspaceId.value }) return
        switchWorkspace(all.first().id)
    }

    /** Reloads everything the screens show (pull to refresh, `resync`, coming back online). */
    suspend fun refresh() {
        _refreshing.value = true
        try {
            // The workspace first: two lists depend on it.
            refreshKey("workspaces")
            coroutineScope { (ALL - "workspaces").map { key -> async { refreshKey(key) } }.awaitAll() }
        } finally {
            _refreshing.value = false
        }
    }

    suspend fun switchWorkspace(id: String) {
        if (id == _workspaceId.value) return
        _workspaceId.value = id
        store.setSettings(store.settings.value.copy(workspace = id))
        _threads.value = cache.read("threads-$id", ListSerializer(BotThread.serializer())) ?: emptyList()
        _folios.value = cache.read("folios-$id", ListSerializer(Folio.serializer())) ?: emptyList()
        scope.launch { refreshKey("threads") }
        scope.launch { refreshKey("folios") }
    }

    /** Refetches a list soon: a burst of events makes one request per list. */
    fun invalidate(vararg keys: String) {
        synchronized(pending) { pending += keys }
        if (flush?.isActive == true) return
        flush = scope.launch {
            delay(300)
            val now = synchronized(pending) { pending.toList().also { pending.clear() } }
            coroutineScope { now.map { async { refreshKey(it) } }.awaitAll() }
        }
    }

    /** What each topic means is web/src/lib/ws.ts. */
    private fun onEvent(ev: BusEvent) {
        when {
            ev.topic == "today" -> invalidate("today")
            ev.topic == "sessions" -> if (ev.type == "updated") {
                val s = runCatching { LpJson.decodeFromJsonElement(Session.serializer(), ev.payload) }.getOrNull()
                if (s == null || s.id.isEmpty()) invalidate("sessions") else {
                    val old = _sessions.value
                    val i = old.indexOfFirst { it.id == s.id }
                    _sessions.value = if (i < 0) listOf(s) + old else old.toMutableList().also { it[i] = s }
                }
            } else invalidate("sessions")
            ev.topic == "bots" -> if (ev.type == "threads") invalidate("threads") else if (ev.type != "queued" && ev.type != "sends") invalidate("bots")
            ev.topic == "channels" -> invalidate("channels")
            ev.topic == "folios" -> invalidate("folios")
            ev.topic == "workspaces" -> invalidate("workspaces")
            ev.topic == "projects" -> invalidate("projects")
        }
    }

    fun session(id: String): Session? = _sessions.value.firstOrNull { it.id == id }

    /** Sign out, or the device was removed: stop following the bus and forget everything. */
    suspend fun wipe() {
        jobs.forEach { it.cancel() }
        jobs = emptyList()
        flush?.cancel()
        _workspaces.value = emptyList(); _projects.value = emptyList(); _sessions.value = emptyList(); _bots.value = emptyList()
        _channels.value = emptyList(); _today.value = Today(); _threads.value = emptyList(); _folios.value = emptyList()
        _workspaceId.value = ""
        _loaded.value = false
        cache.wipe()
    }

    private companion object {
        val ALL = listOf("workspaces", "projects", "sessions", "bots", "channels", "today", "threads", "folios")
    }
}
