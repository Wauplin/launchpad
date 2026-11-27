package dev.launchpad.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.launchpad.android.Graph
import dev.launchpad.android.data.lineOf
import dev.launchpad.android.data.statusOf
import dev.launchpad.android.data.titleOf

/**
 * A route the phone does not draw itself: what Home's lists know about it, and ↗ to its exact page in the browser.
 * Conversations, agents and folios get their own screens; Today cards the phone never acts on stay here.
 */
@Composable
fun HandoffScreen(route: Route) {
    val c = LocalLp.current
    val context = LocalContext.current
    val sessions by Graph.repo.sessions.collectAsStateWithLifecycle()
    val today by Graph.repo.today.collectAsStateWithLifecycle()
    val folios by Graph.repo.folios.collectAsStateWithLifecycle()
    val workspace by Graph.repo.workspaceId.collectAsStateWithLifecycle()
    val (title, lines) = when (route) {
        is Route.Agent -> sessions.firstOrNull { it.id == route.sessionId }.let { s -> (s?.let(::titleOf) ?: route.title.ifEmpty { "Agent" }) to listOfNotNull(s?.let { statusOf(it).first + " · " + since(it.status_since) }, s?.let(::lineOf)) }
        is Route.Conversation -> route.title.ifEmpty { "Conversation" } to emptyList()
        is Route.ChannelChat -> route.title.ifEmpty { "Channel" } to emptyList()
        is Route.FolioReader -> folios.firstOrNull { it.slug == route.slug }.let { f -> (f?.title ?: route.title.ifEmpty { route.slug }) to listOfNotNull(f?.summary, f?.let { "rev ${it.rev.toInt()}" }) }
        is Route.Card -> today.items.firstOrNull { it.id == route.itemId }.let { i -> (i?.title ?: route.title.ifEmpty { "Today" }) to listOfNotNull(i?.detail?.takeIf { it.isNotBlank() }) }
        Route.ThisPhone -> "This phone" to emptyList()
        Route.Search -> "Search" to emptyList()
    }
    Column(Modifier.fillMaxSize().background(c.paper)) {
        ScreenBar(title, onBack = { Graph.nav.back() })
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).navigationBarsPadding().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text(title, fontSize = Type.title, fontWeight = FontWeight.SemiBold, color = c.ink)
            for (line in lines) Text(line, fontSize = Type.body, color = c.ink2)
            Muted("This opens on its exact page in the browser, signed in there.")
            Button(onClick = { openInBrowser(context, webPath(route, workspace)) }, modifier = Modifier.fillMaxWidth().heightIn(min = 52.dp)) { Text("Open in browser ↗") }
        }
    }
}
