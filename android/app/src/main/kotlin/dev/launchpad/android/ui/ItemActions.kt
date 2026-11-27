package dev.launchpad.android.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import dev.launchpad.android.Graph
import dev.launchpad.android.data.AskRow
import dev.launchpad.android.data.Outgoing
import dev.launchpad.android.data.SendState
import dev.launchpad.android.data.Sends
import dev.launchpad.android.data.openQuestion
import dev.launchpad.android.data.quickOptions
import dev.launchpad.android.gen.Actions
import dev.launchpad.android.gen.Session
import dev.launchpad.android.gen.TodayItem
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.launch

/** A snackbar with Undo (Seen, Dismiss…): `AppRoot` shows it and runs `undo` when tapped. */
data class UndoSnack(val message: String, val undo: suspend () -> Unit)

object Snacks {
    val undo = MutableSharedFlow<UndoSnack>(extraBufferCapacity = 4)
}

private fun label(action: String, fallback: String) = Actions.get(action)?.label?.takeIf { it.isNotBlank() } ?: fallback

/**
 * The buttons of a Today item, acted on in place (design.md "Now", D-SHADE): a question's options and Reply, Start
 * and Dismiss of a card, Reply to a report or a channel that asks, Seen of a finished or failed agent, and Open.
 * Every button runs an action of the action list; Merge and Publish are never here (Open ↗ hands them off).
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun ItemActions(item: TodayItem, sessions: List<Session>) {
    val context = LocalContext.current
    val sends = remember(item.id) { mutableStateListOf<Outgoing>() }
    var replying by remember(item.id) { mutableStateOf(false) }
    var question by remember(item.id) { mutableStateOf<AskRow?>(null) }
    val workspace = Graph.repo.workspaceId.value
    val open = { Graph.nav.open(routeOf(item, sessions)) }
    val session = item.session_id
    if (item.kind == "question" && session != null) LaunchedEffect(item.id, item.at) { question = openQuestion(session) }
    val last = sends.lastOrNull()

    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            when (item.kind) {
                "question" -> {
                    val q = question
                    val options = q?.let(::quickOptions) ?: emptyList()
                    if (q != null && session != null && last?.state != SendState.Sent) options.forEachIndexed { i, o ->
                        val answer = { Graph.scope.launch { track(sends, Sends.answer(session, q.requestId, listOf(o))) }; Unit }
                        if (i == 0) Button(onClick = answer, modifier = Modifier.heightIn(min = 44.dp)) { Text(o) } else OutlinedButton(onClick = answer, modifier = Modifier.heightIn(min = 44.dp)) { Text(o) }
                    }
                    if (options.isEmpty()) Button(onClick = open, modifier = Modifier.heightIn(min = 44.dp)) { Text("Answer") }
                    else OutlinedButton(onClick = open, modifier = Modifier.heightIn(min = 44.dp)) { Text("Reply") }
                }
                "finished", "failed" -> {
                    Button(onClick = open, modifier = Modifier.heightIn(min = 44.dp)) { Text("Open") }
                    if (session != null) OutlinedButton(onClick = {
                        Graph.scope.launch {
                            if (Sends.action("session.seen", mapOf("id" to session))) Snacks.undo.tryEmit(UndoSnack("Marked seen") { Sends.action("session.unread", mapOf("id" to session)) })
                        }
                    }, modifier = Modifier.heightIn(min = 44.dp)) { Text("Seen") }
                }
                "proposal" -> when (item.action) {
                    "proposal.reply" -> {
                        Button(onClick = { replying = true }, modifier = Modifier.heightIn(min = 44.dp)) { Text(label("proposal.reply", "Reply")) }
                        OutlinedButton(onClick = { Graph.scope.launch { Sends.action("proposal.dismiss", mapOf("id" to item.record), done = "Dismissed") } }, modifier = Modifier.heightIn(min = 44.dp)) { Text(label("proposal.dismiss", "Dismiss")) }
                    }
                    "proposal.start", null -> {
                        Button(onClick = { Graph.scope.launch { Sends.action("proposal.start", mapOf("id" to item.record), done = "Started") } }, modifier = Modifier.heightIn(min = 44.dp)) { Text(label("proposal.start", "Start")) }
                        OutlinedButton(onClick = { Graph.scope.launch { Sends.action("proposal.dismiss", mapOf("id" to item.record), done = "Dismissed") } }, modifier = Modifier.heightIn(min = 44.dp)) { Text(label("proposal.dismiss", "Dismiss")) }
                        OutlinedButton(onClick = { openInBrowser(context, "/?card=${item.id}&ws=$workspace") }, modifier = Modifier.heightIn(min = 44.dp)) { Text("Details ↗") }
                    }
                    "proposal.dismiss" -> {
                        Button(onClick = { Graph.scope.launch { Sends.action("proposal.dismiss", mapOf("id" to item.record), done = "Marked read") } }, modifier = Modifier.heightIn(min = 44.dp)) { Text("Mark read") }
                        OutlinedButton(onClick = { openInBrowser(context, "/?card=${item.id}&ws=$workspace") }, modifier = Modifier.heightIn(min = 44.dp)) { Text("Details ↗") }
                    }
                    // A ready card (Merge, Publish, Post, Send…) stays the owner's click in the browser.
                    else -> Button(onClick = { openInBrowser(context, "/?card=${item.id}&ws=$workspace") }, modifier = Modifier.heightIn(min = 44.dp)) { Text("Open ↗") }
                }
                "channel_ask" -> {
                    Button(onClick = { replying = true }, modifier = Modifier.heightIn(min = 44.dp)) { Text("Reply") }
                    OutlinedButton(onClick = open, modifier = Modifier.heightIn(min = 44.dp)) { Text("Open") }
                }
                else -> Button(onClick = open, modifier = Modifier.heightIn(min = 44.dp)) { Text("Open") }
            }
        }
        if (replying && last?.state != SendState.Sent) InlineReply { text ->
            val o = if (item.kind == "channel_ask") Sends.channelPost(item.record, text, item.message?.toLong()) else Sends.proposalReply(item.record, text)
            Graph.scope.launch { track(sends, o) }
        }
        last?.let { o -> SendLine(o) { Graph.scope.launch { retry(sends, it, { Graph.repo.invalidate("today") }) } } }
    }
}

/** A one-line reply under a card. */
@Composable
fun InlineReply(onSend: (String) -> Unit) {
    var text by remember { mutableStateOf("") }
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.Bottom, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
        OutlinedTextField(text, { text = it }, Modifier.weight(1f), placeholder = { Text("Reply…") }, maxLines = 4, shape = RoundedCornerShape(12.dp))
        Button(onClick = { val t = text.trim(); text = ""; onSend(t) }, enabled = text.isNotBlank(), modifier = Modifier.heightIn(min = 48.dp)) { Text("Send") }
    }
}
