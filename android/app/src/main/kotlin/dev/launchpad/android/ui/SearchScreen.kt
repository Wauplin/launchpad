package dev.launchpad.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.launchpad.android.Graph
import dev.launchpad.android.data.ConversationKind
import dev.launchpad.android.data.Tone
import dev.launchpad.android.data.agentRow
import dev.launchpad.android.data.agents
import dev.launchpad.android.data.chats
import kotlinx.coroutines.launch
import java.net.URLEncoder

/**
 * ⌕: the workspace's loaded conversations, agents and folios whose title or last line holds the words; a pasted
 * Launchpad link or `lp:` ID opens its screen. Search everything ↗ is the browser's search.
 */
@Composable
fun SearchScreen() {
    val c = LocalLp.current
    val context = LocalContext.current
    val repo = Graph.repo
    val workspace by repo.workspaceId.collectAsStateWithLifecycle()
    val sessions by repo.sessions.collectAsStateWithLifecycle()
    val projects by repo.projects.collectAsStateWithLifecycle()
    val bots by repo.bots.collectAsStateWithLifecycle()
    val threads by repo.threads.collectAsStateWithLifecycle()
    val channels by repo.channels.collectAsStateWithLifecycle()
    val folios by repo.folios.collectAsStateWithLifecycle()
    var query by rememberSaveable { mutableStateOf("") }
    val focus = remember { FocusRequester() }
    LaunchedEffect(Unit) { runCatching { focus.requestFocus() } }
    val words = query.lowercase().split(' ').filter { it.isNotBlank() }
    fun hit(vararg texts: String?) = words.isNotEmpty() && words.all { w -> texts.any { it?.lowercase()?.contains(w) == true } }
    val rows = remember(query, workspace, sessions, bots, threads, channels, projects) {
        (chats(workspace, bots, threads, sessions, channels) + agents(workspace, sessions, projects).map(::agentRow)).filter { hit(it.title, it.line) }.sortedByDescending { it.last }
    }
    val folioHits = remember(query, folios) { folios.filter { hit(it.title, it.summary, it.slug) } }
    Column(Modifier.fillMaxSize().background(c.paper).imePadding()) {
        ScreenBar("Search", onBack = { Graph.nav.back() })
        OutlinedTextField(
            value = query, onValueChange = { query = it }, placeholder = { Text("Chats, agents, folios, or a Launchpad link") },
            singleLine = true, shape = RoundedCornerShape(12.dp), modifier = Modifier.fillMaxWidth().padding(12.dp).focusRequester(focus),
        )
        LazyColumn(Modifier.weight(1f), contentPadding = PaddingValues(bottom = 16.dp)) {
            if (Links.isLaunchpad(query)) item(key = "link") {
                ListRow("Open this link", query.trim(), "", Tone.Running, "↪") { Graph.scope.launch { Links.openResolved(context, query, fromOutside = false) } }
            }
            if (words.isNotEmpty() && rows.isEmpty() && folioHits.isEmpty()) item(key = "none") { Muted("Nothing loaded on this phone matches.", Modifier.padding(16.dp)) }
            items(rows, key = { it.key }) { r ->
                val glyph = when (r.kind) { ConversationKind.Chat -> "💬"; ConversationKind.Channel -> "#"; ConversationKind.Agent -> "◎" }
                ListRow(r.title, r.line, r.label, r.tone, glyph) { Graph.nav.open(routeOf(r)) }
            }
            items(folioHits, key = { "f:${it.id}" }) { f -> ListRow(f.title, f.summary ?: f.kind, "rev ${f.rev.toInt()}", Tone.Idle, "❏") { Graph.nav.open(Route.FolioReader(f.slug, f.title)) } }
            item(key = "everything") {
                OutlinedButton(
                    onClick = { openInBrowser(context, "/chats?ws=$workspace" + if (query.isNotBlank()) "&q=" + URLEncoder.encode(query, "UTF-8") else "") },
                    modifier = Modifier.fillMaxWidth().padding(16.dp).heightIn(min = 48.dp),
                ) { Text("Search everything ↗") }
            }
        }
    }
}
