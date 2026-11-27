package dev.launchpad.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.launchpad.android.Graph
import dev.launchpad.android.data.AskRow
import dev.launchpad.android.data.Outgoing
import dev.launchpad.android.data.Presence
import dev.launchpad.android.data.SendState
import dev.launchpad.android.data.Sends
import dev.launchpad.android.data.TraceLog
import dev.launchpad.android.data.TraceRow
import dev.launchpad.android.data.UserRow
import dev.launchpad.android.data.chatRows
import dev.launchpad.android.data.isRunning
import dev.launchpad.android.data.isUnseen
import dev.launchpad.android.data.openAsk
import dev.launchpad.android.data.statusOf
import dev.launchpad.android.data.thread
import dev.launchpad.android.data.traceRows
import dev.launchpad.android.gen.Session
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** Marks a finished, failed or interrupted session seen once it was on screen for 1.5 s while the owner uses the phone (the web's rule). */
@Composable
fun SeenEffect(s: Session?) {
    val due = s != null && s.isUnseen && (s.status == "waiting" || s.status == "error" || s.status == "interrupted")
    LaunchedEffect(s?.id, s?.status_since, due) {
        if (!due) return@LaunchedEffect
        delay(1500)
        if (Presence.active) Sends.seen(s.id)
    }
}

/** Reports the sessions a screen shows (presence), and nothing once it leaves. */
@Composable
fun ShowsSessions(ids: List<String>) {
    DisposableEffect(ids) {
        Presence.show(ids)
        onDispose { Presence.show(emptyList()) }
    }
}

/** A session's trace while a screen shows it: follows `session/<id>` only meanwhile. */
@Composable
fun rememberTrace(sessionId: String?): TraceLog? {
    val log = remember(sessionId) { sessionId?.takeIf { it.isNotEmpty() }?.let { TraceLog(it) } }
    DisposableEffect(log) {
        log?.start()
        onDispose { log?.stop() }
    }
    return log
}

/** The newest session of a bot's thread (the one a message goes on in), an archived one only when none is left. */
fun threadSession(sessions: List<Session>, botId: String, thread: String): Session? {
    val mine = sessions.filter { it.launch_spec.bot?.id == botId && it.thread == thread }
    return mine.filter { it.status != "closed" }.maxByOrNull { it.started_at } ?: mine.maxByOrNull { it.started_at }
}

/**
 * A bot's thread as a chat (design.md "reply"): the answer streams in, questions are answered in place, the box has
 * focus, sends show Sending… → Sent ✓ (or Not sent · Send again, Uncertain · Check), never sent twice by themselves.
 */
@Composable
fun ConversationScreen(route: Route.Conversation) {
    val c = LocalLp.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val bots by Graph.repo.bots.collectAsStateWithLifecycle()
    val threads by Graph.repo.threads.collectAsStateWithLifecycle()
    val sessions by Graph.repo.sessions.collectAsStateWithLifecycle()
    val workspace by Graph.repo.workspaceId.collectAsStateWithLifecycle()
    val bot = bots.firstOrNull { it.id == route.botId }
    val threadRecord = threads.firstOrNull { it.bot_id == route.botId && it.id == route.thread }
    val session = remember(sessions, route) { threadSession(sessions, route.botId, route.thread) }
    val log = rememberTrace(session?.id)
    val state = log?.state?.collectAsStateWithLifecycle()?.value
    val rows: List<TraceRow> = remember(state) { state?.let { chatRows(traceRows(it)) } ?: emptyList() }
    val ask = remember(state) { state?.let { openAsk(traceRows(it)) }?.takeIf { session?.open_question == true } }
    val outgoing = remember(route.botId, route.thread) { mutableStateListOf<Outgoing>() }
    // A sent message leaves the list once the trace shows it.
    val shown = remember(rows) { rows.filterIsInstance<UserRow>() }
    val pending = outgoing.filter { o -> o.state != SendState.Sent || shown.none { u -> u.clientId == o.key || (u.text.trim() == o.text.trim() && u.ts >= o.at - 120_000) } }
    val list = rememberLazyListState()
    var menu by remember { mutableStateOf(false) }
    ShowsSessions(listOfNotNull(session?.id))
    SeenEffect(session)
    LaunchedEffect(rows.size, pending.size) { val n = rows.size + pending.size; if (n > 0) list.animateScrollToItem(n) }

    val title = threadRecord?.title?.takeIf { it.isNotBlank() } ?: route.title.ifBlank { bot?.settings?.name ?: "Chat" }
    val status = session?.let { s -> if (s.isRunning) "Working…" else statusOf(s).first } ?: ""
    val onSent: suspend (Outgoing) -> Unit = { o ->
        // A thread that cannot go on continues in a new one: follow the message there.
        Sends.receipt(o)?.thread?.takeIf { it.isNotEmpty() && it != route.thread }?.let { t ->
            val nav = Graph.nav
            val i = nav.stack.indexOf(route)
            if (i >= 0) nav.stack[i] = route.copy(thread = t)
        }
        Graph.repo.invalidate("sessions", "threads")
    }
    val answer: (AskRow, List<String>) -> Unit = { a, answers ->
        val s = session
        if (s != null) Graph.scope.launch { track(outgoing, Sends.answer(s.id, a.requestId, answers)) }
    }

    Column(Modifier.fillMaxSize().background(c.paper)) {
        ScreenBar(if (status.isEmpty()) title else "$title · $status", onBack = { Graph.nav.back() }) {
            GlyphButton("⋯", "More", { menu = true })
            DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                DropdownMenuItem(text = { Text("Open in browser ↗") }, onClick = { menu = false; openInBrowser(context, webPath(route, workspace)) })
                DropdownMenuItem(text = { Text("Bot settings ↗") }, onClick = { menu = false; openInBrowser(context, "/bots/${route.botId}/settings?ws=$workspace") })
                if (session != null) DropdownMenuItem(text = { Text("Trace") }, onClick = { menu = false; Graph.nav.open(Route.Agent(session.id, title)) })
            }
        }
        ConnectionBar(Graph.stream.conn.collectAsStateWithLifecycle().value)
        LazyColumn(Modifier.weight(1f).fillMaxWidth(), state = list, contentPadding = PaddingValues(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            if (state?.more == true) item(key = "earlier") {
                TextButton(onClick = { scope.launch { log.loadEarlier() } }, modifier = Modifier.fillMaxWidth()) { Text(if (state.loadingEarlier) "Loading…" else "Load earlier messages") }
            }
            if (session == null && pending.isEmpty()) item(key = "empty") {
                Column(horizontalAlignment = Alignment.CenterHorizontally, modifier = Modifier.fillMaxWidth()) {
                    Muted("Your next message starts this chat with ${bot?.settings?.name ?: "the bot"}.")
                    Starters(bot?.settings?.starters ?: emptyList()) { s -> Graph.scope.launch { track(outgoing, Sends.botMessage(route.botId, route.thread, s), onSent) } }
                }
            }
            if (state != null && !state.loaded) item(key = "loading") { Muted("Loading…") }
            state?.error?.let { e -> item(key = "error") { Text(e, fontSize = Type.small, color = c.danger) } }
            items(rows, key = { it.key }) { TraceRowView(it, answer) }
            items(pending, key = { "o:${it.key}" }) { o -> OutgoingBubble(o) { Graph.scope.launch { retry(outgoing, it, { log?.load() }, onSent) } } }
        }
        val answering = ask != null && ask.questions.none { it.multi_select } && session != null
        Composer(
            draftKey = "bot-${route.botId}-${route.thread}",
            placeholder = if (answering) "Answer…" else "Message ${bot?.settings?.name ?: ""}…",
            enabled = bot != null,
            attachments = !answering,
            onSend = { text, files ->
                if (answering) {
                    // A typed answer answers every question of the card with the same text (the web's rule).
                    Graph.scope.launch { track(outgoing, Sends.answer(session!!.id, ask!!.requestId, List(ask.questions.size) { text })) }
                } else {
                    Graph.scope.launch { track(outgoing, Sends.botMessage(route.botId, route.thread, text, files), onSent) }
                }
                true
            },
            extra = { if (answering) Text("Answering: ${ask.questions.firstOrNull()?.question?.lineSequence()?.firstOrNull() ?: ""}", Modifier.padding(horizontal = 12.dp, vertical = 4.dp), fontSize = Type.tiny, color = c.attn, maxLines = 1) },
        )
    }
}
