package dev.launchpad.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.launchpad.android.Graph
import dev.launchpad.android.data.str
import dev.launchpad.android.data.Outgoing
import dev.launchpad.android.data.Presence
import dev.launchpad.android.data.SendState
import dev.launchpad.android.data.Sends
import dev.launchpad.android.gen.ChannelMessage
import dev.launchpad.android.gen.ChannelPage
import dev.launchpad.android.net.HomeException
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.serialization.Serializable
import java.net.URLEncoder

@Serializable
private data class Messages(val messages: List<ChannelMessage> = emptyList(), val more: Boolean = false)

/**
 * A bot channel or DM: the newest messages (earlier ones on demand), new ones as Home announces them, the owner's
 * post (`channel.post`, answering the bot that asks when there is one), and Mark read once shown.
 */
@Composable
fun ChannelScreen(route: Route.ChannelChat) {
    val c = LocalLp.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val workspace by Graph.repo.workspaceId.collectAsStateWithLifecycle()
    val id = URLEncoder.encode(route.channelId, "UTF-8")
    var page by remember(route.channelId) { mutableStateOf<ChannelPage?>(null) }
    val messages = remember(route.channelId) { mutableStateListOf<ChannelMessage>() }
    var more by remember(route.channelId) { mutableStateOf(false) }
    var error by remember(route.channelId) { mutableStateOf<String?>(null) }
    val outgoing = remember(route.channelId) { mutableStateListOf<Outgoing>() }
    val list = rememberLazyListState()

    suspend fun loadNewer() {
        try {
            val last = messages.lastOrNull()?.id?.toLong()
            if (last == null) {
                val p = Graph.home.get("/api/channels/$id", ChannelPage.serializer())
                page = p
                messages.clear(); messages.addAll(p.messages); more = p.more
            } else {
                val m = Graph.home.get("/api/channels/$id/messages?since=$last", Messages.serializer())
                messages.addAll(m.messages.filter { n -> messages.none { it.id == n.id } })
            }
            error = null
        } catch (e: HomeException) { error = e.message }
    }
    suspend fun loadEarlier() {
        val first = messages.firstOrNull()?.id?.toLong() ?: return
        try {
            val p = Graph.home.get("/api/channels/$id?before=$first", ChannelPage.serializer())
            messages.addAll(0, p.messages.filter { n -> messages.none { it.id == n.id } })
            more = p.more
        } catch (e: HomeException) { Graph.toast(e.message ?: "Home did not answer") }
    }
    LaunchedEffect(route.channelId) {
        loadNewer()
        launch { Graph.stream.resync.collect { loadNewer() } }
        Graph.stream.events.collect { ev ->
            if (ev.topic != "channels") return@collect
            val about = ev.payload.str("id")
            if (about == null || about == route.channelId) loadNewer()
        }
    }
    // Read once the newest message was on screen for 1.5 s while the owner uses the phone.
    val newest = messages.lastOrNull()?.id?.toLong()
    LaunchedEffect(newest) {
        if (newest == null) return@LaunchedEffect
        delay(1500)
        if (Presence.active) Sends.channelRead(route.channelId, newest)
    }
    val shown = messages.filter { it.author.kind == "owner" }
    val pending = outgoing.filter { o -> o.state != SendState.Sent || shown.none { it.text.trim() == o.text.trim() && it.created_at >= o.at - 120_000 } }
    LaunchedEffect(messages.size, pending.size) { val n = messages.size + pending.size; if (n > 0) list.animateScrollToItem(n) }
    // A bot that asks the owner: the post answers its newest ask.
    val asking = messages.lastOrNull { it.ask_owner }?.takeIf { a -> messages.none { it.author.kind == "owner" && it.id > a.id } }
    val channel = page?.channel
    val title = channel?.let { ch -> ch.title.ifBlank { if (ch.kind == "dm") ch.name else "#${ch.name}" } } ?: route.title.ifBlank { "Channel" }

    Column(Modifier.fillMaxSize().background(c.paper)) {
        ScreenBar(title, onBack = { Graph.nav.back() }) {
            GlyphButton("↗", "Open in browser", { openInBrowser(context, webPath(route, workspace)) })
        }
        ConnectionBar(Graph.stream.conn.collectAsStateWithLifecycle().value)
        LazyColumn(Modifier.weight(1f).fillMaxWidth(), state = list, contentPadding = PaddingValues(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            if (more) item(key = "earlier") { TextButton(onClick = { scope.launch { loadEarlier() } }, modifier = Modifier.fillMaxWidth()) { Text("Load earlier messages") } }
            if (page == null && error == null) item(key = "loading") { Muted("Loading…") }
            error?.let { e -> item(key = "error") { Text(e, fontSize = Type.small, color = c.danger) } }
            channel?.purpose?.takeIf { it.isNotBlank() }?.let { p -> item(key = "purpose") { Muted(p) } }
            items(messages, key = { "m:${it.id.toLong()}" }) { m -> MessageRow(m) }
            items(pending, key = { "o:${it.key}" }) { o -> OutgoingBubble(o) { Graph.scope.launch { retry(outgoing, it, { loadNewer() }) } } }
        }
        Composer(
            draftKey = "channel-${route.channelId}",
            placeholder = if (asking != null) "Answer ${asking.author.name}…" else "Message $title…",
            attachments = false,
            enabled = channel?.archived != true,
            onSend = { text, _ ->
                Graph.scope.launch { track(outgoing, Sends.channelPost(route.channelId, text, asking?.id?.toLong())) { loadNewer() } }
                true
            },
        )
    }
}

@Composable
private fun MessageRow(m: ChannelMessage) {
    val c = LocalLp.current
    val owner = m.author.kind == "owner"
    Column(Modifier.fillMaxWidth().padding(start = if (owner) 40.dp else 0.dp), horizontalAlignment = if (owner) Alignment.End else Alignment.Start) {
        Text(
            listOfNotNull(if (owner) "You" else m.author.name, ago(m.created_at), if (m.ask_owner) "asks you" else null).joinToString(" · "),
            fontSize = Type.tiny, color = if (m.ask_owner) c.attn else c.ink3, fontWeight = if (m.ask_owner) FontWeight.SemiBold else FontWeight.Normal,
        )
        Box(Modifier.clip(RoundedCornerShape(12.dp)).background(if (owner) c.tealSoft else c.sunk).padding(horizontal = 12.dp, vertical = 8.dp)) {
            Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                if (owner) LinkedText(m.text) else Markdown(m.text)
                for (f in m.files) Text("📎 ${f.name}", fontSize = Type.small, color = c.ink2)
            }
        }
    }
}
