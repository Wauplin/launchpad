package dev.launchpad.android.data

import dev.launchpad.android.Graph
import dev.launchpad.android.gen.ImageAttachment
import dev.launchpad.android.gen.Question
import dev.launchpad.android.gen.SessionEvent
import dev.launchpad.android.gen.SessionEventPage
import dev.launchpad.android.net.HomeException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull

// A session's trace on the phone: the events Home keeps (`SessionEvent`, read tail-first) and the live text of the
// answer being written. The rules are the web's (web/src/lib/sessionStore.ts): a live event with `seq` 0 is a delta
// of text that the final message replaces; a gap in `seq` reloads.

private const val PAGE = 200

data class TraceState(
    val events: List<SessionEvent> = emptyList(),
    /** Live text of answers not finished yet, by item id. */
    val deltas: Map<String, String> = emptyMap(),
    val loaded: Boolean = false,
    val error: String? = null,
    /** Earlier events exist on Home. */
    val more: Boolean = false,
    val loadingEarlier: Boolean = false,
)

private fun JsonElement.kind(): String = str("kind") ?: ""

/** The trace of one session while a screen shows it: `start()` follows `session/<id>`, `stop()` lets it go. */
class TraceLog(val sessionId: String, private val scope: CoroutineScope = Graph.scope) {
    private val _state = MutableStateFlow(TraceState())
    val state: StateFlow<TraceState> = _state
    private var jobs: List<Job> = emptyList()
    private var unwatch: (() -> Unit)? = null
    private val mutex = Mutex()

    fun start() {
        if (jobs.isNotEmpty()) return
        unwatch = Graph.stream.watch("session/$sessionId")
        jobs = listOf(
            scope.launch {
                Graph.stream.events.collect { ev ->
                    if (ev.topic != "session/$sessionId") return@collect
                    val e = runCatching { LpJson.decodeFromJsonElement(SessionEvent.serializer(), ev.payload) }.getOrNull() ?: return@collect
                    receive(e)
                }
            },
            scope.launch { Graph.stream.resync.collect { load() } },
            scope.launch { load() },
        )
    }

    fun stop() {
        jobs.forEach { it.cancel() }
        jobs = emptyList()
        unwatch?.invoke()
        unwatch = null
    }

    /** The newest events (tail-first), or what came after the ones already here. */
    suspend fun load() = mutex.withLock {
        val now = _state.value
        try {
            if (now.loaded && now.events.isNotEmpty()) {
                val after = now.events.last().seq.toLong()
                val fresh = Graph.home.get("/api/sessions/$sessionId/events?after=$after", ListSerializer(SessionEvent.serializer()))
                _state.value = settle(_state.value.copy(events = (_state.value.events + fresh).distinctBy { it.seq }.sortedBy { it.seq }, error = null), fresh)
            } else {
                val page = Graph.home.get("/api/sessions/$sessionId/events/page?tail=$PAGE", SessionEventPage.serializer())
                _state.value = TraceState(events = page.events, loaded = true, more = (page.events.firstOrNull()?.seq ?: 1.0) > 1.0)
            }
        } catch (e: HomeException) {
            _state.value = _state.value.copy(error = e.message, loaded = _state.value.loaded)
        }
    }

    suspend fun loadEarlier() {
        val first = _state.value.events.firstOrNull()?.seq?.toLong() ?: return
        if (!_state.value.more || _state.value.loadingEarlier) return
        _state.value = _state.value.copy(loadingEarlier = true)
        try {
            val page = Graph.home.get("/api/sessions/$sessionId/events/page?before=$first&limit=$PAGE", SessionEventPage.serializer())
            mutex.withLock {
                val all = (page.events + _state.value.events).distinctBy { it.seq }.sortedBy { it.seq }
                _state.value = _state.value.copy(events = all, more = page.events.isNotEmpty() && (all.firstOrNull()?.seq ?: 1.0) > 1.0, loadingEarlier = false)
            }
        } catch (e: HomeException) {
            _state.value = _state.value.copy(loadingEarlier = false)
            Graph.toast(e.message ?: "Home did not answer")
        }
    }

    private suspend fun receive(e: SessionEvent) {
        val st = _state.value
        if (!st.loaded) return
        if (e.seq == 0.0) {
            val wrapped = e.body
            if (wrapped.kind() == "subagent_event") return
            if (wrapped.kind() == "assistant_delta") {
                val id = wrapped.str("item_id") ?: return
                _state.value = st.copy(deltas = st.deltas + (id to ((st.deltas[id] ?: "") + (wrapped.str("text") ?: ""))))
            }
            return
        }
        val last = st.events.lastOrNull()?.seq ?: 0.0
        if (e.seq <= last) return
        if (last != 0.0 && e.seq != last + 1) {
            load()
            return
        }
        _state.value = settle(st.copy(events = st.events + e), listOf(e))
    }

    /** Drops the live text a final event replaces. */
    private fun settle(st: TraceState, fresh: List<SessionEvent>): TraceState {
        var deltas = st.deltas
        for (e in fresh) {
            when (e.body.kind()) {
                "assistant_message" -> e.body.str("item_id")?.let { deltas = deltas - it }
                "turn_end" -> deltas = emptyMap()
            }
        }
        return if (deltas === st.deltas) st else st.copy(deltas = deltas)
    }
}

// ---------------------------------------------------------------- rows

sealed interface TraceRow { val key: String }
/** A message to the agent: the owner's, or another sender's (`from`). `clientId`: the send it came from. */
data class UserRow(override val key: String, val text: String, val images: List<ImageAttachment>, val from: String?, val clientId: String, val ts: Double) : TraceRow
data class AssistantRow(override val key: String, val text: String, val streaming: Boolean = false) : TraceRow
/** One tool call on one line. */
data class ToolRow(override val key: String, val name: String, val summary: String, val done: Boolean, val error: Boolean) : TraceRow
/** A question; `answers` null while it is open. */
data class AskRow(override val key: String, val requestId: String, val questions: List<Question>, val answers: List<String>?) : TraceRow
/** A notice, an error, a stopped or failed turn. */
data class NoteRow(override val key: String, val text: String, val danger: Boolean) : TraceRow
/** The end of a turn. */
data class TurnRow(override val key: String, val seconds: Long?) : TraceRow
/** In a chat: the work behind an answer, folded into one line. */
data class StepsRow(override val key: String, val rows: List<TraceRow>) : TraceRow

private fun strings(e: JsonElement?): List<String> = (e as? JsonArray)?.mapNotNull { (it as? JsonPrimitive)?.contentOrNull } ?: emptyList()

/** Every event as a row, oldest first (the Trace tab). Reasoning, plans and sub-agent events are left out. */
fun traceRows(state: TraceState): List<TraceRow> {
    val rows = ArrayList<TraceRow>()
    val tools = HashMap<String, Int>()
    val asks = HashMap<String, Int>()
    for (e in state.events) {
        val b = e.body
        val key = "e${e.seq.toLong()}"
        when (b.kind()) {
            "user_message" -> {
                val images = runCatching { LpJson.decodeFromJsonElement(ListSerializer(ImageAttachment.serializer()), (b as JsonObject)["images"] ?: JsonArray(emptyList())) }.getOrDefault(emptyList())
                val from = b.obj("from")?.let { it.str("name") }?.takeIf { it.isNotBlank() }
                rows += UserRow(key, b.str("text") ?: "", images, from, b.str("client_id") ?: "", e.ts)
            }
            "assistant_message" -> rows += AssistantRow(key, b.str("text") ?: "")
            "tool_call" -> {
                tools[b.str("item_id") ?: key] = rows.size
                rows += ToolRow(key, b.str("name") ?: "Tool", (b.str("summary") ?: "").lineSequence().firstOrNull()?.take(200) ?: "", done = false, error = false)
            }
            "tool_result" -> tools[b.str("item_id") ?: ""]?.let { i -> (rows[i] as? ToolRow)?.let { rows[i] = it.copy(done = true, error = b.bool("is_error") == true) } }
            "file_change" -> rows += ToolRow(key, b.str("change")?.replaceFirstChar { it.uppercase() } ?: "Edit", b.str("path") ?: "", done = true, error = false)
            "ask_user" -> {
                val questions = runCatching { LpJson.decodeFromJsonElement(ListSerializer(Question.serializer()), (b as JsonObject)["questions"] ?: JsonArray(emptyList())) }.getOrDefault(emptyList())
                asks[b.str("request_id") ?: ""] = rows.size
                rows += AskRow(key, b.str("request_id") ?: "", questions, null)
            }
            "ask_answered" -> asks[b.str("request_id") ?: ""]?.let { i -> (rows[i] as? AskRow)?.let { rows[i] = it.copy(answers = strings((b as JsonObject)["answers"])) } }
            "turn_end" -> {
                val error = b.str("error")
                if (!error.isNullOrBlank()) rows += NoteRow("${key}x", "Failed: ${error.lineSequence().first().take(300)}", true)
                else if (b.bool("interrupted") == true) rows += NoteRow("${key}x", "Stopped", false)
                val ms = ((b as? JsonObject)?.get("duration_ms") as? JsonPrimitive)?.contentOrNull?.toDoubleOrNull()
                rows += TurnRow(key, ms?.let { (it / 1000).toLong() })
                // A tool that never got its result is over with its turn.
                for (i in rows.indices) (rows[i] as? ToolRow)?.takeIf { !it.done }?.let { rows[i] = it.copy(done = true) }
            }
            "error" -> rows += NoteRow(key, b.str("message") ?: "Error", true)
            "notice" -> rows += NoteRow(key, b.str("text") ?: "", false)
        }
    }
    for ((id, text) in state.deltas) if (text.isNotBlank()) rows += AssistantRow("d$id", text, streaming = true)
    return rows
}

/**
 * The same rows as a chat (web/src/features/chat/chatRows.ts): per turn the messages, the questions, the notes and
 * the last answer; the tool calls and the texts before it fold into one steps line.
 */
fun chatRows(rows: List<TraceRow>): List<TraceRow> {
    val out = ArrayList<TraceRow>()
    var turn = ArrayList<TraceRow>()
    fun flush() {
        if (turn.isEmpty()) return
        val answer = turn.indexOfLast { it is AssistantRow }
        val steps = ArrayList<TraceRow>()
        val kept = ArrayList<Pair<Int, TraceRow>>()
        turn.forEachIndexed { i, r ->
            when {
                r is ToolRow || (r is AssistantRow && i != answer) -> steps += r
                r is TurnRow -> {}
                else -> kept += i to r
            }
        }
        var placed = steps.isEmpty()
        for ((i, r) in kept) {
            // The steps line sits before the answer (or before what follows the work).
            if (!placed && (i == answer || (r !is UserRow && i > (answer.takeIf { it >= 0 } ?: Int.MAX_VALUE)))) {
                out += StepsRow("s${steps.first().key}", steps)
                placed = true
            }
            out += r
        }
        if (!placed) out += StepsRow("s${steps.first().key}", steps)
        turn = ArrayList()
    }
    for (r in rows) {
        if (r is UserRow && turn.any { it !is UserRow }) flush()
        turn += r
        if (r is TurnRow) flush()
    }
    flush()
    return out
}

/** The question a session waits on: the last one asked and not answered. */
fun openAsk(rows: List<TraceRow>): AskRow? = rows.lastOrNull { it is AskRow }?.let { it as AskRow }?.takeIf { it.answers == null }

/** The open question of a session, read from its newest events (Today's waiting card, a notification's buttons). */
suspend fun openQuestion(sessionId: String): AskRow? = try {
    val page = Graph.home.get("/api/sessions/${java.net.URLEncoder.encode(sessionId, "UTF-8")}/events/page?tail=60", SessionEventPage.serializer())
    openAsk(traceRows(TraceState(events = page.events, loaded = true)))
} catch (_: HomeException) {
    null
}

/**
 * The buttons a question offers in place (at most 3): the options of a single question that takes one answer.
 * Several questions, a multiple choice or more options need the conversation (Reply).
 */
fun quickOptions(ask: AskRow): List<String> {
    val q = ask.questions.singleOrNull() ?: return emptyList()
    if (q.multi_select || q.options.isEmpty() || q.options.size > 3) return emptyList()
    return q.options.map { it.label }
}
