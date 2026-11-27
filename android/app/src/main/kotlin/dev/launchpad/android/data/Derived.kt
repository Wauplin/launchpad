package dev.launchpad.android.data

import dev.launchpad.android.gen.Bot
import dev.launchpad.android.gen.BotThread
import dev.launchpad.android.gen.Channel
import dev.launchpad.android.gen.Project
import dev.launchpad.android.gen.Session
import dev.launchpad.android.gen.Today
import dev.launchpad.android.gen.TodayItem

// What the lists show of Home's records. The rules are the web app's (web/src/lib/workspace.ts, sessionStatus.ts,
// chatList.ts, botThread.ts), so the phone and the laptop agree.

enum class Tone { Running, Needs, Done, Failed, Idle }

/** The waiting items of one workspace, in Home's order (the filter `GET /api/today?workspace=` applies). */
fun todayOf(today: Today, workspace: String): List<TodayItem> = today.items.filter { it.workspaces.isEmpty() || workspace in it.workspaces }

/** Whether a session belongs to a workspace (`scopeOf` in web/src/lib/workspace.ts). */
fun inWorkspace(s: Session, workspace: String, projects: List<Project>): Boolean {
    if (workspace.isEmpty()) return false
    val mine = projects.filter { it.workspace_id == workspace }
    fun hasProject(id: String?, cwd: String = "") = (id != null && mine.any { it.id == id }) || (cwd.isNotEmpty() && mine.any { cwd == it.path || cwd.startsWith(it.path + "/") })
    val target = s.launch_spec.target
    val specProject = target.str("kind").let { kind -> if (kind == "project" || kind == "predraft") target.str("project_id") else null }
    return hasProject(s.project_id, s.cwd) || s.launch_spec.workspace_id == workspace || hasProject(specProject)
}

val Session.isBot: Boolean get() = launch_spec.bot != null
/** The thread of a bot session (sessions from before bots had several threads are in `main`). */
val Session.thread: String get() = launch_spec.bot?.thread?.takeIf { it.isNotEmpty() } ?: "main"
val Session.isRunning: Boolean get() = status == "starting" || status == "working" || status == "background"
val Session.isUnseen: Boolean get() = seen_at < status_since
val Session.isSleeping: Boolean get() = status == "waiting" && !open_question && sleep != null
private val Session.failed: Boolean get() = (status == "error" && capacity_retry_at == null) || status == "interrupted"

/** A session's group in lists: Home's `group` (`Session::group`), with the web's rule as a fallback for older Homes. */
fun groupOf(s: Session): String {
    s.group?.takeIf { it.isNotEmpty() }?.let { return it }
    if (s.status == "closed") return "archived"
    if ((s.status == "waiting" && (s.open_question || (s.isUnseen && !s.isSleeping))) || ((s.status == "error" || s.status == "interrupted") && s.isUnseen)) return "needs_you"
    if (s.isSleeping) return "sleeping"
    return when (s.status) {
        "waiting" -> "done"
        "starting", "working", "background" -> "running"
        "quota_paused" -> "quota_paused"
        "stopped" -> "stopped"
        "interrupted" -> "interrupted"
        "error" -> "failed"
        "in_terminal" -> "in_terminal"
        else -> "done"
    }
}

/** The list sections of agents, in the laptop's order, with their titles. */
val GROUPS: List<Pair<String, String>> = listOf(
    "needs_you" to "Needs you", "running" to "Running", "quota_paused" to "Quota paused", "sleeping" to "Sleeping",
    "failed" to "Failed", "interrupted" to "Interrupted", "done" to "Done", "stopped" to "Stopped", "in_terminal" to "In terminal",
)

/** A short status, in the words and colors of the laptop's status pill (`threadStatus` in chatList.ts). */
fun statusOf(s: Session): Pair<String, Tone> = when (s.status) {
    "starting" -> "Starting" to Tone.Running
    "working" -> "Working" to Tone.Running
    "background" -> "Background" to Tone.Running
    "waiting" -> if (s.open_question) "Question" to Tone.Needs else if (s.sleep != null) "Sleeping" to Tone.Idle else "Done" to (if (s.isUnseen) Tone.Needs else Tone.Done)
    "quota_paused" -> "Quota paused" to Tone.Needs
    "interrupted" -> "Interrupted" to Tone.Failed
    "error" -> if (s.capacity_retry_at != null) "Retry pending" to Tone.Idle else "Failed" to Tone.Failed
    "stopped" -> "Stopped" to Tone.Idle
    "in_terminal" -> "In terminal" to Tone.Idle
    else -> "Archived" to Tone.Idle
}

/** One line of what a session is doing or said last. */
fun lineOf(s: Session): String {
    val text = s.last_message?.lineSequence()?.firstOrNull { it.isNotBlank() }?.trim()
    return when {
        s.status == "error" && !s.failure.isNullOrBlank() -> s.failure.lineSequence().first().trim()
        !text.isNullOrEmpty() -> text
        else -> statusOf(s).first
    }.take(160)
}

fun titleOf(s: Session): String = s.title.ifBlank { s.prompt.lineSequence().firstOrNull { it.isNotBlank() }?.trim()?.take(80) ?: "Agent" }

enum class ConversationKind { Chat, Channel, Agent }

/** A row of Recent conversations and of the Chats and Agents lists. */
data class ConversationRow(
    val kind: ConversationKind,
    /** Stable in lists. */
    val key: String,
    val title: String,
    val line: String,
    val label: String,
    val tone: Tone,
    /** Unix ms of its newest activity. */
    val last: Double,
    /** Chats list section: Needs you, Unread, Running, Sleeping, Failed, Done, Channels. */
    val section: String,
    val botId: String? = null,
    val thread: String? = null,
    val channelId: String? = null,
    val sessionId: String? = null,
)

private fun urgency(s: Session): Int = when {
    s.open_question -> 0
    s.status == "quota_paused" -> 1
    s.isUnseen && ((s.status == "waiting" && s.sleep == null) || s.failed) -> 2
    s.failed -> 3
    s.isRunning -> 4
    s.isSleeping -> 5
    else -> 6
}

private fun chatSection(s: Session?): String = when {
    s == null -> "Done"
    s.open_question -> "Needs you"
    s.isUnseen && !s.open_question && ((s.status == "waiting" && s.sleep == null) || s.failed) -> "Unread"
    s.isRunning -> "Running"
    s.status == "quota_paused" -> "Quota paused"
    s.isSleeping -> "Sleeping"
    s.failed -> "Failed"
    else -> "Done"
}

val CHAT_SECTIONS = listOf("Needs you", "Unread", "Running", "Quota paused", "Sleeping", "Failed", "Done", "Channels")

/**
 * The workspace's bot conversations (one row per `BotThread.conversation`, its most urgent state and newest activity)
 * and its channels and DMs, newest first. Archived ones are left out.
 */
fun chats(workspace: String, bots: List<Bot>, threads: List<BotThread>, sessions: List<Session>, channels: List<Channel>): List<ConversationRow> {
    val mine = bots.filter { it.workspace_id == workspace }.associateBy { it.id }
    // The open session of each thread: its newest that is not archived.
    val current = HashMap<String, Session>()
    for (s in sessions.sortedBy { it.started_at }) {
        val bot = s.launch_spec.bot ?: continue
        if (bot.id !in mine) continue
        current["${bot.id}\n${s.thread}"] = s
    }
    current.entries.removeAll { it.value.status == "closed" }
    val rows = ArrayList<ConversationRow>()
    for ((conversation, group) in threads.filter { it.archived_at == null && it.bot_id in mine }.groupBy { it.conversation.ifEmpty { "${it.bot_id}:${it.id}" } }) {
        val live = group.mapNotNull { t -> current["${t.bot_id}\n${t.id}"] }
        val shown = live.minWithOrNull(compareBy<Session> { urgency(it) }.thenByDescending { it.status_since })
        // The thread that opens: the shown session's, else the conversation's first.
        val first = group.minBy { it.created_at }
        val open = shown?.let { s -> group.firstOrNull { it.bot_id == s.launch_spec.bot?.id && it.id == s.thread } } ?: first
        val bot = mine[open.bot_id] ?: continue
        val (label, tone) = shown?.let(::statusOf) ?: ("" to Tone.Idle)
        val last = maxOf(live.maxOfOrNull { it.status_since } ?: 0.0, group.maxOf { it.created_at })
        rows += ConversationRow(
            ConversationKind.Chat, "chat:$conversation", first.title.ifBlank { bot.settings.name.ifBlank { bot.slug } },
            shown?.let(::lineOf) ?: bot.settings.name, label, tone, last, chatSection(shown), botId = bot.id, thread = open.id, sessionId = shown?.id,
        )
    }
    for (c in channels.filter { it.workspace_id == workspace && !it.archived }) {
        val unread = c.unread > 0 || c.attention > 0
        rows += ConversationRow(
            ConversationKind.Channel, "channel:${c.id}", c.title.ifBlank { if (c.kind == "dm") c.name else "#${c.name}" },
            c.last_text?.lineSequence()?.firstOrNull()?.take(160) ?: c.purpose, if (c.attention > 0) "Asks you" else if (unread) "Unread" else "",
            if (c.attention > 0) Tone.Needs else if (unread) Tone.Running else Tone.Idle, c.last_at ?: c.created_at,
            if (c.attention > 0) "Needs you" else if (unread) "Unread" else "Channels", channelId = c.id,
        )
    }
    return rows.sortedByDescending { it.last }
}

/** The workspace's agents (sessions that are not bot threads), newest first; archived ones are left out. */
fun agents(workspace: String, sessions: List<Session>, projects: List<Project>): List<Session> =
    sessions.filter { !it.isBot && it.status != "closed" && inWorkspace(it, workspace, projects) }.sortedByDescending { it.status_since }

fun agentRow(s: Session): ConversationRow {
    val (label, tone) = statusOf(s)
    return ConversationRow(ConversationKind.Agent, "agent:${s.id}", titleOf(s), lineOf(s), label, tone, s.status_since, groupOf(s), sessionId = s.id)
}
