package dev.launchpad.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Tab
import androidx.compose.material3.PrimaryTabRow
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.launchpad.android.Graph
import dev.launchpad.android.data.AskRow
import dev.launchpad.android.data.Outgoing
import dev.launchpad.android.data.SendState
import dev.launchpad.android.data.Sends
import dev.launchpad.android.data.UserRow
import dev.launchpad.android.data.isBot
import dev.launchpad.android.data.isRunning
import dev.launchpad.android.data.lineOf
import dev.launchpad.android.data.openAsk
import dev.launchpad.android.data.statusOf
import dev.launchpad.android.data.titleOf
import dev.launchpad.android.data.traceRows
import dev.launchpad.android.gen.Session
import kotlinx.coroutines.launch
import java.text.DateFormat
import java.util.Date

/**
 * An agent (design.md "answer" and "progress"): Summary with the current step, the Trace opened tail-first and
 * followed live, the question card above Reply, Stop (asks first). Diffs, Merge and Files open in the browser (↗).
 */
@Composable
fun AgentScreen(route: Route.Agent) {
    val c = LocalLp.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val sessions by Graph.repo.sessions.collectAsStateWithLifecycle()
    val projects by Graph.repo.projects.collectAsStateWithLifecycle()
    val workspace by Graph.repo.workspaceId.collectAsStateWithLifecycle()
    val s = sessions.firstOrNull { it.id == route.sessionId }
    val log = rememberTrace(route.sessionId)
    val state = log?.state?.collectAsStateWithLifecycle()?.value
    val rows = remember(state) { state?.let(::traceRows) ?: emptyList() }
    val ask = remember(rows, s?.open_question) { openAsk(rows)?.takeIf { s?.open_question != false } }
    var tab by rememberSaveable { mutableStateOf(1) }
    var menu by remember { mutableStateOf(false) }
    var stopping by remember { mutableStateOf(false) }
    val outgoing = remember(route.sessionId) { mutableStateListOf<Outgoing>() }
    val shown = remember(rows) { rows.filterIsInstance<UserRow>() }
    val pending = outgoing.filter { o -> o.state != SendState.Sent || shown.none { u -> u.clientId == o.key || (u.text.trim() == o.text.trim() && u.ts >= o.at - 120_000) } }
    ShowsSessions(listOf(route.sessionId))
    SeenEffect(s)
    val title = s?.let(::titleOf) ?: route.title.ifBlank { "Agent" }
    val answer: (AskRow, List<String>) -> Unit = { a, answers -> Graph.scope.launch { track(outgoing, Sends.answer(route.sessionId, a.requestId, answers)) } }

    Column(Modifier.fillMaxSize().background(c.paper)) {
        ScreenBar(title, onBack = { Graph.nav.back() }) {
            if (s != null && s.isRunning) TextButton(onClick = { stopping = true }) { Text("Stop", color = c.danger) }
            GlyphButton("⋯", "More", { menu = true })
            DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                DropdownMenuItem(text = { Text("Changes ↗") }, onClick = { menu = false; openInBrowser(context, "/agents?session=${route.sessionId}&view=changes&ws=$workspace") })
                DropdownMenuItem(text = { Text("Open in browser ↗") }, onClick = { menu = false; openInBrowser(context, webPath(route, workspace)) })
                if (s != null && s.isBot) DropdownMenuItem(text = { Text("Open the chat") }, onClick = { menu = false; Graph.nav.open(Route.Conversation(s.launch_spec.bot?.id ?: "", s.launch_spec.bot?.thread?.ifEmpty { "main" } ?: "main", title)) })
                if (s != null && s.status != "closed" && !s.isRunning) DropdownMenuItem(text = { Text("Mark unread") }, onClick = {
                    menu = false
                    scope.launch { Sends.action("session.unread", mapOf("id" to s.id), done = "Marked unread") }
                })
            }
        }
        ConnectionBar(Graph.stream.conn.collectAsStateWithLifecycle().value)
        PrimaryTabRow(selectedTabIndex = tab, containerColor = c.chrome) {
            Tab(selected = tab == 0, onClick = { tab = 0 }, text = { Text("Summary") }, modifier = Modifier.heightIn(min = 48.dp))
            Tab(selected = tab == 1, onClick = { tab = 1 }, text = { Text("Trace") }, modifier = Modifier.heightIn(min = 48.dp))
        }
        if (tab == 0) Summary(s, projects.firstOrNull { it.id == s?.project_id }?.name, Modifier.weight(1f))
        else {
            val list = rememberLazyListState()
            LaunchedEffect(rows.size, pending.size) { val n = rows.size + pending.size; if (n > 0) list.animateScrollToItem(n) }
            LazyColumn(Modifier.weight(1f).fillMaxWidth(), state = list, contentPadding = PaddingValues(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                if (state?.more == true) item(key = "earlier") {
                    TextButton(onClick = { scope.launch { log.loadEarlier() } }, modifier = Modifier.fillMaxWidth()) { Text(if (state.loadingEarlier) "Loading…" else "Load earlier events") }
                }
                if (state != null && !state.loaded) item(key = "loading") { Muted("Loading the trace…") }
                state?.error?.let { e -> item(key = "error") { Text(e, fontSize = Type.small, color = c.danger) } }
                // The open question sits above Reply, not in the list.
                items(rows.filter { it !== ask }, key = { it.key }) { TraceRowView(it, answer) }
                items(pending, key = { "o:${it.key}" }) { o -> OutgoingBubble(o) { Graph.scope.launch { retry(outgoing, it, { log?.load() }) } } }
            }
        }
        if (ask != null) Column(Modifier.padding(horizontal = 12.dp, vertical = 6.dp)) { QuestionCard(ask, answer) }
        val answering = ask != null && ask.questions.none { it.multi_select }
        Composer(
            draftKey = "agent-${route.sessionId}",
            placeholder = if (answering) "Answer…" else "Reply…",
            enabled = s != null && s.status != "closed",
            attachments = !answering,
            focus = false,
            onSend = { text, files ->
                if (answering) Graph.scope.launch { track(outgoing, Sends.answer(route.sessionId, ask!!.requestId, List(ask.questions.size) { text })) }
                else Graph.scope.launch { track(outgoing, Sends.sessionInput(route.sessionId, text, files)) { Graph.repo.invalidate("sessions") } }
                tab = 1
                true
            },
        )
    }
    if (stopping) AlertDialog(
        onDismissRequest = { stopping = false },
        title = { Text("Stop this agent?") },
        text = { Text("It ends the current turn. You can send a message to continue later.") },
        confirmButton = { TextButton(onClick = { stopping = false; scope.launch { Sends.action("session.stop", mapOf("id" to route.sessionId), done = "Stopped") } }) { Text("Stop", color = c.danger) } },
        dismissButton = { TextButton(onClick = { stopping = false }) { Text("Cancel") } },
    )
}

@Composable
private fun Summary(s: Session?, folder: String?, modifier: Modifier) {
    val c = LocalLp.current
    val context = LocalContext.current
    val workspace by Graph.repo.workspaceId.collectAsStateWithLifecycle()
    Column(modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        if (s == null) { Muted("This agent is not in the lists of this phone yet."); return@Column }
        val (label, tone) = statusOf(s)
        Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Dot(toneColor(tone))
            Text("$label · ${since(s.status_since)}", fontSize = Type.body, fontWeight = FontWeight.SemiBold, color = toneColor(tone))
        }
        Text(lineOf(s), fontSize = Type.body, color = c.ink)
        if (!s.failure.isNullOrBlank()) Text(s.failure, fontSize = Type.small, color = c.danger)
        s.last_message?.takeIf { it.isNotBlank() && it.trim() != lineOf(s) }?.let { Card { Markdown(it.take(4000)) } }
        val fmt = DateFormat.getDateTimeInstance(DateFormat.MEDIUM, DateFormat.SHORT)
        Fact("Folder", folder ?: s.cwd.substringAfterLast('/').ifEmpty { "—" })
        Fact("Started", fmt.format(Date(s.started_at.toLong())))
        s.model?.let { Fact("Model", listOfNotNull(it, s.effort).joinToString(" · ")) }
        s.usage.cost_usd?.let { Fact("Cost", "$" + String.format(java.util.Locale.US, "%.2f", it)) }
        s.sleep?.let { Fact("Sleeping", "until it is woken") }
        Muted("Diffs, Merge and Files are in the browser.")
        OutlinedButton(onClick = { openInBrowser(context, "/agents?session=${s.id}&view=changes&ws=$workspace") }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Changes ↗") }
    }
}

@Composable
private fun Fact(label: String, value: String) {
    val c = LocalLp.current
    Row {
        Text(label, Modifier.width(96.dp), fontSize = Type.small, color = c.ink3)
        Text(value, fontSize = Type.small, color = c.ink)
    }
}
