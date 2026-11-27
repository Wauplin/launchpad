package dev.launchpad.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.launchpad.android.Graph
import dev.launchpad.android.data.CHAT_SECTIONS
import dev.launchpad.android.data.ConversationKind
import dev.launchpad.android.data.ConversationRow
import dev.launchpad.android.data.GROUPS
import dev.launchpad.android.data.Tone
import dev.launchpad.android.data.agentRow
import dev.launchpad.android.data.agents
import dev.launchpad.android.data.chats
import dev.launchpad.android.data.groupOf
import dev.launchpad.android.data.isBot
import dev.launchpad.android.data.isRunning
import dev.launchpad.android.data.lineOf
import dev.launchpad.android.data.thread
import dev.launchpad.android.data.titleOf
import dev.launchpad.android.data.todayOf
import dev.launchpad.android.gen.Bot
import dev.launchpad.android.gen.Folio
import dev.launchpad.android.gen.Session
import dev.launchpad.android.gen.TodayItem
import dev.launchpad.android.net.Conn
import kotlinx.coroutines.launch

/** Where a row leads. Chats and agents get their own screens; so do folios; the rest hands off to the browser. */
fun routeOf(row: ConversationRow): Route = when (row.kind) {
    ConversationKind.Chat -> Route.Conversation(row.botId ?: "", row.thread ?: "main", row.title)
    ConversationKind.Channel -> Route.ChannelChat(row.channelId ?: "", row.title)
    ConversationKind.Agent -> Route.Agent(row.sessionId ?: "", row.title)
}

/** The screen of a Today item: its agent or conversation, its channel, else its card in the browser. */
fun routeOf(item: TodayItem, sessions: List<Session>): Route {
    val session = item.session_id?.let { id -> sessions.firstOrNull { it.id == id } }
    return when {
        item.kind == "channel_ask" -> Route.ChannelChat(item.record, item.title)
        item.kind in listOf("question", "finished", "failed") && session != null ->
            if (session.isBot) Route.Conversation(session.launch_spec.bot?.id ?: "", session.thread, item.title) else Route.Agent(session.id, item.title)
        item.kind in listOf("question", "finished", "failed") && item.session_id != null -> Route.Agent(item.session_id, item.title)
        else -> Route.Card(item.id, item.title)
    }
}

private fun glyph(kind: ConversationKind) = when (kind) {
    ConversationKind.Chat -> "💬"
    ConversationKind.Channel -> "#"
    ConversationKind.Agent -> "◎"
}

private val KIND_LABEL = mapOf(
    "question" to "Question", "finished" to "Finished", "failed" to "Failed", "local_pr" to "Local PR", "proposal" to "Proposal", "outbox" to "Outbox",
    "suggestion" to "Suggestion", "routine" to "Routine", "runner" to "Runner", "home_update" to "Home update", "cli_update" to "Agent CLI update",
    "channel_ask" to "Asks you", "oracle" to "Oracle",
)

/** Where the Ask box sends: a bot of the workspace, or a new agent with Home's defaults. */
sealed interface AskTarget {
    data class ToBot(val bot: Bot) : AskTarget
    data object NewAgent : AskTarget
    val key: String get() = when (this) { is ToBot -> "bot:${bot.id}"; NewAgent -> "agent" }
    val label: String get() = when (this) { is ToBot -> "@${bot.settings.name.ifBlank { bot.slug }}"; NewAgent -> "New agent" }
}

/** The workspace's targets: its Ask target first (the oldest bot with admin Launchpad access, else the oldest), the other bots, then New agent. */
fun askTargets(bots: List<Bot>, workspace: String): List<AskTarget> {
    val mine = bots.filter { it.workspace_id == workspace && !it.paused }.sortedBy { it.created_at }
    val first = mine.firstOrNull { it.settings.launchpad == "admin" } ?: mine.firstOrNull()
    return (listOfNotNull(first) + mine.filter { it !== first }).map { AskTarget.ToBot(it) } + AskTarget.NewAgent
}

/**
 * Now (navigation model C): what waits, what runs, recent conversations and folios on one screen; the scope chips
 * turn it into a full list; the Ask box stays at the bottom. `onAsk` sends the box's text (conversations wire it).
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun NowScreen(onAsk: (AskTarget, String) -> Unit) {
    val c = LocalLp.current
    val context = LocalContext.current
    val nav = Graph.nav
    val repo = Graph.repo
    val workspace by repo.workspaceId.collectAsStateWithLifecycle()
    val workspaces by repo.workspaces.collectAsStateWithLifecycle()
    val today by repo.today.collectAsStateWithLifecycle()
    val sessions by repo.sessions.collectAsStateWithLifecycle()
    val projects by repo.projects.collectAsStateWithLifecycle()
    val bots by repo.bots.collectAsStateWithLifecycle()
    val threads by repo.threads.collectAsStateWithLifecycle()
    val channels by repo.channels.collectAsStateWithLifecycle()
    val folios by repo.folios.collectAsStateWithLifecycle()
    val refreshing by repo.refreshing.collectAsStateWithLifecycle()
    val loaded by repo.loaded.collectAsStateWithLifecycle()
    val conn by Graph.stream.conn.collectAsStateWithLifecycle()

    val waiting = remember(today, workspace) { todayOf(today, workspace) }
    val chatRows = remember(workspace, bots, threads, sessions, channels) { chats(workspace, bots, threads, sessions, channels) }
    val agentList = remember(workspace, sessions, projects) { agents(workspace, sessions, projects) }
    val folioList = remember(folios) { folios.filter { it.archived_at == null }.sortedByDescending { it.updated_at } }
    var menu by remember { mutableStateOf(false) }

    Column(Modifier.fillMaxSize().background(c.paper)) {
        // Top bar: the workspace (with the others' waiting counts), the connection dot, search, This phone.
        Column(Modifier.background(c.chrome).statusBarsPadding()) {
            Row(Modifier.fillMaxWidth().heightIn(min = 52.dp).padding(horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                Box(Modifier.weight(1f)) {
                    val name = workspaces.firstOrNull { it.id == workspace }?.name ?: "Launchpad"
                    TextButton(onClick = { menu = true }, modifier = Modifier.heightIn(min = 48.dp).semantics { contentDescription = "Workspace $name. Switch workspace" }) {
                        Text("$name ▾", fontSize = Type.title, fontWeight = FontWeight.SemiBold, color = c.ink, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                    DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                        for (w in workspaces) {
                            // The count is the length of Home's own list for that workspace.
                            val n = todayOf(today, w.id).size
                            DropdownMenuItem(
                                text = { Text(if (n > 0) "${w.name} · $n waiting" else w.name, fontWeight = if (w.id == workspace) FontWeight.SemiBold else FontWeight.Normal) },
                                onClick = { menu = false; Graph.scope.launch { repo.switchWorkspace(w.id) } },
                            )
                        }
                    }
                }
                val (dot, state) = when (conn) {
                    Conn.Live -> c.tealInk to "Live"
                    Conn.Connecting, Conn.Paused -> c.line to "Connecting"
                    else -> c.attn to "Not live"
                }
                GlyphButton("●", "Connection: $state", { Graph.stream.retryNow(); Graph.toast(if (conn == Conn.Live) "Live · ${Graph.store.link.value.space}" else "$state · ${Graph.store.link.value.space}") }, color = dot)
                GlyphButton("⌕", "Search in the browser", { openInBrowser(context, "/chats?ws=$workspace") })
                GlyphButton("⋯", "This phone", { nav.open(Route.ThisPhone) })
            }
            // Scope chips: each turns the screen into that full list.
            Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 12.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                for (scope in Scope.entries) {
                    val label = if (scope == Scope.Now && waiting.isNotEmpty()) "Now · ${waiting.size}" else scope.label
                    FilterChip(selected = nav.scope == scope, onClick = { nav.scope = scope }, label = { Text(label, fontSize = Type.small) }, modifier = Modifier.heightIn(min = 44.dp))
                }
            }
            HorizontalDivider(color = c.line2)
        }
        ConnectionBar(conn)

        PullToRefreshBox(isRefreshing = refreshing && loaded, onRefresh = { Graph.stream.retryNow(); Graph.scope.launch { repo.refresh() } }, modifier = Modifier.weight(1f)) {
            LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 12.dp)) {
                if (!loaded) {
                    item { Muted(if (conn is Conn.Offline) "No data yet: this phone has not reached Home." else "Loading…", Modifier.padding(16.dp)) }
                } else when (nav.scope) {
                    Scope.Now -> {
                        item(key = "waiting") {
                            SectionTitle(if (waiting.isEmpty()) "Waiting for you" else "Waiting for you · ${waiting.size}")
                            WaitingCard(waiting, sessions)
                        }
                        val live = agentList.filter { it.isRunning }
                        item(key = "live-title") { SectionTitle("Live", if (live.size > 2) "All live" else null) { nav.scope = Scope.Agents } }
                        if (live.isEmpty()) item(key = "live-none") { Muted("No agent is running.", Modifier.padding(horizontal = 16.dp, vertical = 4.dp)) }
                        items(live.take(2), key = { "live:${it.id}" }) { LiveRow(it) }
                        val recent = (chatRows + agentList.map(::agentRow)).sortedByDescending { it.last }
                        item(key = "recent-title") { SectionTitle("Recent conversations", if (recent.size > 3) "All" else null) { nav.scope = Scope.Chats } }
                        if (recent.isEmpty()) item(key = "recent-none") { Muted("No conversation yet. Ask a bot below.", Modifier.padding(horizontal = 16.dp, vertical = 4.dp)) }
                        items(recent.take(3), key = { "recent:${it.key}" }) { Row2(it) }
                        item(key = "folios") {
                            SectionTitle("Folios", if (folioList.size > 3) "All" else null) { nav.scope = Scope.Folios }
                            if (folioList.isEmpty()) Muted("No folio yet.", Modifier.padding(horizontal = 16.dp, vertical = 4.dp))
                            Row(Modifier.horizontalScroll(rememberScrollState()).padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                for (f in folioList.take(3)) FolioCard(f)
                            }
                        }
                    }
                    Scope.Chats -> {
                        if (chatRows.isEmpty()) item { Muted("No conversation in this workspace yet.", Modifier.padding(16.dp)) }
                        for (section in CHAT_SECTIONS) {
                            val rows = chatRows.filter { it.section == section }
                            if (rows.isEmpty()) continue
                            item(key = "s:$section") { SectionTitle("$section · ${rows.size}") }
                            items(rows, key = { it.key }) { Row2(it) }
                        }
                    }
                    Scope.Agents -> {
                        if (agentList.isEmpty()) item { Muted("No agent in this workspace yet.", Modifier.padding(16.dp)) }
                        val grouped = agentList.groupBy(::groupOf)
                        for ((group, title) in GROUPS) {
                            val rows = grouped[group] ?: continue
                            item(key = "g:$group") { SectionTitle("$title · ${rows.size}") }
                            items(rows, key = { it.id }) { Row2(agentRow(it)) }
                        }
                    }
                    Scope.Folios -> {
                        if (folioList.isEmpty()) item { Muted("No folio in this workspace yet.", Modifier.padding(16.dp)) }
                        items(folioList, key = { it.id }) { f ->
                            val unsent = f.unsent.toInt()
                            ListRow(f.title, f.summary ?: f.kind, if (unsent > 0) "$unsent unsent" else "rev ${f.rev.toInt()}", if (unsent > 0) Tone.Needs else Tone.Idle, "❏") { nav.open(Route.FolioReader(f.slug, f.title)) }
                        }
                    }
                }
            }
        }
        AskBox(askTargets(bots, workspace), onAsk)
    }
}

/** The state of the link when it is not Live (design.md "States"). */
@Composable
fun ConnectionBar(conn: Conn) {
    val context = LocalContext.current
    val last by Graph.stream.lastData.collectAsStateWithLifecycle()
    when (conn) {
        is Conn.Offline -> Banner("Offline" + ((conn.lastData ?: last)?.let { " · last data ${ago(it.toDouble())}" } ?: ""), "Retry now") { Graph.stream.retryNow() }
        Conn.Starting -> Banner("Home is starting (the Space wakes up). Retrying…", "Retry now") { Graph.stream.retryNow() }
        Conn.Updating -> Banner("Home is updating · writes wait until it is back")
        Conn.AuthExpired -> Banner("Hugging Face no longer accepts the login.", "Sign in again") { Graph.signIn.start(context, reauth = true) }
        is Conn.Incompatible -> Banner(conn.message, "Check again") { Graph.stream.retryNow() }
        Conn.Live, Conn.Connecting, Conn.Paused -> {}
    }
}

/** The first of Today's items, in Today's order, with N of M; Later › shows the next. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun WaitingCard(waiting: List<TodayItem>, sessions: List<Session>) {
    val c = LocalLp.current
    // Items the owner put off, by id: they come back after the others.
    var later by rememberSaveable { mutableStateOf(listOf<String>()) }
    if (waiting.isEmpty()) {
        Card(Modifier.fillMaxWidth().padding(horizontal = 16.dp)) { Muted("Nothing waits for you ✓") }
        return
    }
    val order = waiting.filter { it.id !in later } + later.mapNotNull { id -> waiting.firstOrNull { it.id == id } }
    val item = order.first()
    Card(Modifier.fillMaxWidth().padding(horizontal = 16.dp), border = c.attnLine) {
        Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Row(verticalAlignment = Alignment.Top) {
                Text(item.title, Modifier.weight(1f), fontSize = Type.body, fontWeight = FontWeight.SemiBold, color = c.ink, maxLines = 3, overflow = TextOverflow.Ellipsis)
                Spacer(Modifier.width(8.dp))
                Text("${waiting.indexOf(item) + 1} of ${waiting.size}", fontSize = Type.tiny, color = c.ink3)
            }
            Text(listOfNotNull(KIND_LABEL[item.kind], ago(item.at).takeIf { it.isNotEmpty() }).joinToString(" · "), fontSize = Type.tiny, color = c.attn)
            if (item.detail.isNotBlank()) Text(item.detail, fontSize = Type.small, color = c.ink2, maxLines = 4, overflow = TextOverflow.Ellipsis)
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(onClick = { Graph.nav.open(routeOf(item, sessions)) }, modifier = Modifier.heightIn(min = 44.dp)) { Text("Open") }
                if (waiting.size > 1) OutlinedButton(onClick = { later = (later - item.id) + item.id }, modifier = Modifier.heightIn(min = 44.dp)) { Text("Later ›") }
            }
        }
    }
}

/** A running agent: its current step and how long it has run. */
@Composable
private fun LiveRow(s: Session) {
    val c = LocalLp.current
    Row(
        Modifier.fillMaxWidth().clickable(role = Role.Button) { Graph.nav.open(Route.Agent(s.id, titleOf(s))) }.heightIn(min = 56.dp).padding(horizontal = 16.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Dot(toneColor(Tone.Running))
        Column(Modifier.weight(1f)) {
            Text(titleOf(s), fontSize = Type.body, fontWeight = FontWeight.Medium, color = c.ink, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(lineOf(s), fontSize = Type.small, color = c.ink3, maxLines = 1, overflow = TextOverflow.Ellipsis)
            LinearProgressIndicator(Modifier.fillMaxWidth().padding(top = 4.dp).height(3.dp).clip(RoundedCornerShape(2.dp)), color = c.work, trackColor = c.sunk)
        }
        Text(since(s.status_since), fontSize = Type.tiny, color = c.ink3)
    }
}

@Composable
private fun Row2(row: ConversationRow) {
    ListRow(row.title, row.line, row.label, row.tone, glyph(row.kind)) { Graph.nav.open(routeOf(row)) }
}

@Composable
private fun FolioCard(f: Folio) {
    val c = LocalLp.current
    val unsent = f.unsent.toInt()
    Card(Modifier.widthIn(min = 140.dp, max = 220.dp).clickable(role = Role.Button) { Graph.nav.open(Route.FolioReader(f.slug, f.title)) }) {
        Column {
            Text(f.title, fontSize = Type.small, fontWeight = FontWeight.SemiBold, color = c.ink, maxLines = 2, overflow = TextOverflow.Ellipsis)
            Text(if (unsent > 0) "$unsent unsent marks" else "rev ${f.rev.toInt()} · ${ago(f.updated_at)}", fontSize = Type.tiny, color = if (unsent > 0) c.attn else c.ink3)
        }
    }
}

/** "Ask or start…": one box for anything new, its target one tap away (the chip cycles through bots and New agent). */
@Composable
fun AskBox(targets: List<AskTarget>, onAsk: (AskTarget, String) -> Unit) {
    val c = LocalLp.current
    val settings by Graph.store.settings.collectAsStateWithLifecycle()
    var text by rememberSaveable { mutableStateOf("") }
    val target = targets.firstOrNull { it.key == settings.askTarget } ?: targets.firstOrNull() ?: AskTarget.NewAgent
    Column(Modifier.background(c.chrome).navigationBarsPadding().imePadding()) {
        HorizontalDivider(color = c.line2)
        Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp, vertical = 6.dp), verticalAlignment = Alignment.Bottom, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            Column(Modifier.weight(1f)) {
                TextButton(
                    onClick = {
                        val next = targets.getOrNull((targets.indexOf(target) + 1) % targets.size.coerceAtLeast(1)) ?: target
                        Graph.scope.launch { Graph.store.setSettings(Graph.store.settings.value.copy(askTarget = next.key)) }
                    },
                    modifier = Modifier.heightIn(min = 44.dp).semantics { contentDescription = "Send to ${target.label}. Change target" },
                ) { Text("${target.label} ⇄", fontSize = Type.small, maxLines = 1, overflow = TextOverflow.Ellipsis) }
                OutlinedTextField(
                    value = text, onValueChange = { text = it }, placeholder = { Text("Ask or start…", fontSize = Type.body) },
                    modifier = Modifier.fillMaxWidth(), maxLines = 5, shape = RoundedCornerShape(12.dp),
                )
            }
            Button(onClick = { val t = text.trim(); text = ""; onAsk(target, t) }, enabled = text.isNotBlank(), modifier = Modifier.heightIn(min = 48.dp)) { Text("Send") }
        }
    }
}
