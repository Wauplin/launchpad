package dev.launchpad.android.ui

import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AssistChip
import androidx.compose.material3.Button
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
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
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.launchpad.android.Graph
import dev.launchpad.android.data.AskRow
import dev.launchpad.android.data.AssistantRow
import dev.launchpad.android.data.NoteRow
import dev.launchpad.android.data.Outgoing
import dev.launchpad.android.data.Presence
import dev.launchpad.android.data.SendState
import dev.launchpad.android.data.Sends
import dev.launchpad.android.data.StepsRow
import dev.launchpad.android.data.ToolRow
import dev.launchpad.android.data.TraceRow
import dev.launchpad.android.data.TurnRow
import dev.launchpad.android.data.UserRow
import dev.launchpad.android.gen.FileAttachment
import dev.launchpad.android.net.HomeException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

// The parts every conversation shares: the rows of a trace or chat, the question card, the state of a send, and the
// composer (draft kept on the phone per conversation, attachments uploaded to Home once).

/** One row. `onAnswer`: a question's answers (one per question). */
@Composable
fun TraceRowView(row: TraceRow, onAnswer: (AskRow, List<String>) -> Unit) {
    val c = LocalLp.current
    when (row) {
        is UserRow -> Column(Modifier.fillMaxWidth().padding(start = 40.dp), horizontalAlignment = Alignment.End) {
            if (row.from != null) Text(row.from, fontSize = Type.tiny, color = c.ink3)
            Box(Modifier.clip(RoundedCornerShape(12.dp)).background(if (row.from == null) c.tealSoft else c.sunk).padding(horizontal = 12.dp, vertical = 8.dp)) {
                Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    if (row.text.isNotBlank()) LinkedText(row.text)
                    for (img in row.images) RemoteImage(img.data_url, img.name)
                }
            }
        }
        is AssistantRow -> Column(Modifier.fillMaxWidth()) {
            Markdown(row.text)
            if (row.streaming) Text("Writing…", fontSize = Type.tiny, color = c.work)
        }
        is ToolRow -> ToolLine(row)
        is StepsRow -> StepsLine(row)
        is AskRow -> QuestionCard(row, onAnswer)
        is NoteRow -> Text(row.text, fontSize = Type.small, color = if (row.danger) c.danger else c.ink3)
        is TurnRow -> Text(row.seconds?.let { "— turn ended · ${if (it < 60) "${it}s" else "${it / 60} min"} —" } ?: "— turn ended —", Modifier.fillMaxWidth(), fontSize = Type.tiny, color = c.ink3)
    }
}

@Composable
private fun ToolLine(row: ToolRow) {
    val c = LocalLp.current
    val mark = when { !row.done -> "…"; row.error -> "✕"; else -> "✓" }
    Text(
        "$mark ${row.name} ${row.summary}", Modifier.fillMaxWidth(), fontSize = Type.small, fontFamily = FontFamily.Monospace,
        color = if (row.error) c.danger else c.ink3, maxLines = 1, overflow = TextOverflow.Ellipsis,
    )
}

/** The work behind an answer, folded into one line; a tap unfolds it. */
@Composable
private fun StepsLine(row: StepsRow) {
    val c = LocalLp.current
    var open by remember(row.key) { mutableStateOf(false) }
    val tools = row.rows.count { it is ToolRow }
    val running = row.rows.any { it is ToolRow && !it.done }
    Column(Modifier.fillMaxWidth()) {
        Text(
            (if (open) "▾ " else "▸ ") + (if (running) "Working · " else "") + "$tools step${if (tools == 1) "" else "s"}",
            Modifier.clickable(role = Role.Button) { open = !open }.heightIn(min = 36.dp).padding(vertical = 8.dp),
            fontSize = Type.small, color = if (running) c.work else c.ink3,
        )
        if (open) Column(Modifier.padding(start = 12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            for (r in row.rows) when (r) {
                is ToolRow -> ToolLine(r)
                is AssistantRow -> Text(r.text.lineSequence().firstOrNull { it.isNotBlank() }?.take(200) ?: "", fontSize = Type.small, color = c.ink2, maxLines = 2, overflow = TextOverflow.Ellipsis)
                else -> {}
            }
        }
    }
}

/** A question with its options as buttons; a free answer goes through the composer (it answers while one is open). */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun QuestionCard(ask: AskRow, onAnswer: (AskRow, List<String>) -> Unit) {
    val c = LocalLp.current
    val picked = remember(ask.key) { mutableStateListOf<MutableList<String>>().apply { repeat(ask.questions.size) { add(mutableListOf()) } } }
    var version by remember(ask.key) { mutableStateOf(0) }
    Card(Modifier.fillMaxWidth(), border = if (ask.answers == null) c.attnLine else c.line2) {
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            ask.questions.forEachIndexed { i, q ->
                if (q.header.isNotBlank()) Text(q.header, fontSize = Type.tiny, color = c.attn, fontWeight = FontWeight.SemiBold)
                Markdown(q.question)
                if (ask.answers == null) FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    for (o in q.options) {
                        val on = version >= 0 && o.label in picked[i]
                        val pick: () -> Unit = {
                            if (q.multi_select) { if (on) picked[i].remove(o.label) else picked[i].add(o.label); version++ }
                            else if (ask.questions.size == 1) onAnswer(ask, listOf(o.label))
                            else { picked[i].clear(); picked[i].add(o.label); version++ }
                        }
                        if (on) Button(onClick = pick, modifier = Modifier.heightIn(min = 44.dp)) { Text(o.label) }
                        else OutlinedButton(onClick = pick, modifier = Modifier.heightIn(min = 44.dp).semantics { if (o.description.isNotBlank()) contentDescription = "${o.label}: ${o.description}" }) { Text(o.label) }
                    }
                }
            }
            if (ask.answers != null) Text("Answered: " + ask.answers.joinToString(" · "), fontSize = Type.small, color = c.ok)
            else if (ask.questions.size > 1 || ask.questions.any { it.multi_select }) {
                val ready = version >= 0 && picked.all { it.isNotEmpty() }
                Button(onClick = { onAnswer(ask, picked.map { it.joinToString(", ") }) }, enabled = ready, modifier = Modifier.heightIn(min = 44.dp)) { Text("Answer") }
            } else Muted("Or type your own answer below.")
        }
    }
}

/** The state of one of the owner's sends, under its bubble (design.md "reply"). */
@Composable
fun SendLine(o: Outgoing, onRetry: (Outgoing) -> Unit) {
    val c = LocalLp.current
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End, verticalAlignment = Alignment.CenterVertically) {
        when (o.state) {
            SendState.Sending -> Text("Sending…", fontSize = Type.tiny, color = c.ink3)
            SendState.Sent -> Text("Sent ✓", fontSize = Type.tiny, color = c.ok)
            SendState.NotSent -> {
                Text("Not sent" + (o.error?.let { " · $it" } ?: ""), Modifier.weight(1f, fill = false), fontSize = Type.tiny, color = c.danger, maxLines = 2, overflow = TextOverflow.Ellipsis)
                TextButton(onClick = { onRetry(o) }) { Text("Send again", fontSize = Type.small) }
            }
            SendState.Uncertain -> {
                Text("Uncertain", fontSize = Type.tiny, color = c.attn)
                // A protected send asks Home with the same key; another one only reloads what Home shows.
                TextButton(onClick = { onRetry(o) }) { Text("Check", fontSize = Type.small) }
            }
        }
    }
}

/** The owner's message that Home has not shown in the trace yet, with its state. */
@Composable
fun OutgoingBubble(o: Outgoing, onRetry: (Outgoing) -> Unit) {
    val c = LocalLp.current
    Column(Modifier.fillMaxWidth().padding(start = 40.dp), horizontalAlignment = Alignment.End) {
        if (o.text.isNotBlank()) Box(Modifier.clip(RoundedCornerShape(12.dp)).background(c.tealSoft).padding(horizontal = 12.dp, vertical = 8.dp)) { LinkedText(o.text) }
        SendLine(o, onRetry)
    }
}

/**
 * Sends an [Outgoing] and follows it in `list` (a screen's sends): Sending, then Sent, Not sent or Uncertain. Retry
 * (Send again, Check) repeats it with the same key and body; an unprotected Check only reloads (`reload`), never
 * sends it again by itself: its button then says Send again, an explicit choice of the owner.
 */
suspend fun track(list: MutableList<Outgoing>, o: Outgoing, onSent: suspend (Outgoing) -> Unit = {}) {
    fun put(x: Outgoing) { val i = list.indexOfFirst { it.key == x.key }; if (i < 0) list.add(x) else list[i] = x }
    put(o.copy(state = SendState.Sending, error = null))
    val done = Sends.deliver(o)
    put(done)
    if (done.state == SendState.Sent) onSent(done)
}

/** Retry of a send by the owner: see [track]. */
suspend fun retry(list: MutableList<Outgoing>, o: Outgoing, reload: suspend () -> Unit, onSent: suspend (Outgoing) -> Unit = {}) {
    if (o.state == SendState.Uncertain && !o.protected) {
        reload()
        val i = list.indexOfFirst { it.key == o.key }
        if (i >= 0) list[i] = o.copy(state = SendState.NotSent, error = "Check the conversation above: it may have arrived")
        return
    }
    track(list, o, onSent)
}

// ---------------------------------------------------------------- the composer

/** Reads a picked or shared file and uploads it to Home once. */
suspend fun uploadUri(context: Context, uri: Uri): FileAttachment = withContext(Dispatchers.IO) {
    val resolver = context.contentResolver
    val type = resolver.getType(uri) ?: "application/octet-stream"
    val name = runCatching {
        resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { cur -> if (cur.moveToFirst()) cur.getString(0) else null }
    }.getOrNull() ?: uri.lastPathSegment ?: "file"
    val bytes = resolver.openInputStream(uri)?.use { it.readBytes() } ?: throw HomeException(dev.launchpad.android.net.Failure.Other, "Cannot read $name", sent = false)
    if (bytes.size > 25 * 1024 * 1024) throw HomeException(dev.launchpad.android.net.Failure.Other, "$name is larger than 25 MB", sent = false)
    Sends.upload(name, type, bytes)
}

/**
 * The message box of a conversation: its draft is kept on the phone (`draftKey`) until sent, it has focus when the
 * screen opens, attachments are uploaded as they are picked. `onSend` returns true once the send is handed over.
 */
@Composable
fun Composer(
    draftKey: String,
    placeholder: String,
    onSend: suspend (String, List<FileAttachment>) -> Boolean,
    enabled: Boolean = true,
    attachments: Boolean = true,
    focus: Boolean = true,
    extra: @Composable () -> Unit = {},
) {
    val c = LocalLp.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var text by remember(draftKey) { mutableStateOf(Graph.cache.readText("draft-$draftKey") ?: "") }
    val files = remember(draftKey) { mutableStateListOf<FileAttachment>() }
    var uploading by remember { mutableStateOf(0) }
    var busy by remember { mutableStateOf(false) }
    val focuser = remember { FocusRequester() }
    LaunchedEffect(draftKey) { if (focus) runCatching { focuser.requestFocus() } }
    val pick = rememberLauncherForActivityResult(ActivityResultContracts.GetMultipleContents()) { uris ->
        for (uri in uris) scope.launch {
            uploading++
            try { files += uploadUri(context, uri) } catch (e: HomeException) { Graph.toast(e.message ?: "Upload failed") } finally { uploading-- }
        }
    }
    Column(Modifier.background(c.chrome).navigationBarsPadding().imePadding()) {
        HorizontalDivider(color = c.line2)
        extra()
        if (files.isNotEmpty() || uploading > 0) Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 8.dp), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            for (f in files.toList()) AssistChip(onClick = { files.remove(f) }, label = { Text("${f.name} ✕", fontSize = Type.small, maxLines = 1) }, modifier = Modifier.semantics { contentDescription = "Remove ${f.name}" })
            if (uploading > 0) AssistChip(onClick = {}, label = { Text("Uploading…", fontSize = Type.small) })
        }
        Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp, vertical = 6.dp), verticalAlignment = Alignment.Bottom, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            if (attachments) GlyphButton("＋", "Attach files", { pick.launch("*/*") })
            OutlinedTextField(
                value = text,
                onValueChange = {
                    text = it
                    Presence.input()
                    scope.launch { Graph.cache.writeText("draft-$draftKey", it) }
                },
                placeholder = { Text(placeholder, fontSize = Type.body) },
                modifier = Modifier.weight(1f).widthIn(min = 120.dp).focusRequester(focuser),
                maxLines = 6, shape = RoundedCornerShape(12.dp), enabled = enabled,
            )
            Button(
                onClick = {
                    val t = text.trim()
                    val sent = files.toList()
                    busy = true
                    scope.launch {
                        try {
                            if (onSend(t, sent)) {
                                text = ""
                                files.clear()
                                Graph.cache.writeText("draft-$draftKey", "")
                            }
                        } finally { busy = false }
                    }
                },
                enabled = enabled && !busy && uploading == 0 && (text.isNotBlank() || files.isNotEmpty()),
                modifier = Modifier.heightIn(min = 48.dp),
            ) { Text("Send") }
        }
    }
}

/** A bot's quick starters, as chips (an empty chat). */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun Starters(starters: List<String>, onPick: (String) -> Unit) {
    val c = LocalLp.current
    if (starters.isEmpty()) return
    FlowRow(Modifier.fillMaxWidth().padding(16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        for (s in starters) Box(
            Modifier.clip(RoundedCornerShape(16.dp)).border(1.dp, c.line, RoundedCornerShape(16.dp)).clickable(role = Role.Button) { onPick(s) }.heightIn(min = 44.dp).padding(horizontal = 12.dp, vertical = 10.dp),
        ) { Text(s, fontSize = Type.small, color = c.ink2, maxLines = 2, overflow = TextOverflow.Ellipsis) }
    }
}
