package dev.launchpad.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.launchpad.android.data.Tone

/** Sizes of the web app (11/12/13/15/18), in sp so the system font scale applies. */
object Type {
    val tiny = 11.sp
    val small = 12.sp
    val body = 14.sp
    val title = 16.sp
    val big = 20.sp
}

@Composable
fun toneColor(tone: Tone): Color = LocalLp.current.let { c ->
    when (tone) {
        Tone.Running -> c.work
        Tone.Needs -> c.attn
        Tone.Done -> c.ok
        Tone.Failed -> c.danger
        Tone.Idle -> c.line
    }
}

/** A status dot; the row's text says the state too, so it is hidden from TalkBack. */
@Composable
fun Dot(color: Color, modifier: Modifier = Modifier) {
    Box(modifier.size(10.dp).clip(CircleShape).background(color).clearAndSetSemantics {})
}

/** A text-glyph button (‹ ⌕ ⋯ ↗) with a 48 dp target and a spoken name. */
@Composable
fun GlyphButton(glyph: String, description: String, onClick: () -> Unit, modifier: Modifier = Modifier, color: Color = LocalLp.current.ink2) {
    Box(
        modifier.defaultMinSize(48.dp, 48.dp).clip(CircleShape).clickable(role = Role.Button, onClick = onClick).semantics { contentDescription = description },
        contentAlignment = Alignment.Center,
    ) {
        Text(glyph, fontSize = 20.sp, color = color, modifier = Modifier.clearAndSetSemantics {})
    }
}

/** The bar of a screen above Now: back, its title, then its own buttons. */
@Composable
fun ScreenBar(title: String, onBack: () -> Unit, actions: @Composable RowScope.() -> Unit = {}) {
    val c = LocalLp.current
    Column(Modifier.background(c.chrome).statusBarsPadding()) {
        Row(Modifier.fillMaxWidth().heightIn(min = 52.dp).padding(horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            GlyphButton("‹", "Back", onBack)
            Text(title, Modifier.weight(1f).semantics { heading() }, fontSize = Type.title, fontWeight = FontWeight.SemiBold, color = c.ink, maxLines = 2, overflow = TextOverflow.Ellipsis)
            actions()
        }
        HorizontalDivider(color = c.line2)
    }
}

@Composable
fun SectionTitle(text: String, action: String? = null, onAction: () -> Unit = {}) {
    val c = LocalLp.current
    Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 4.dp, top = 6.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(text.uppercase(), Modifier.weight(1f).padding(vertical = 8.dp).semantics { heading() }, fontSize = Type.tiny, fontWeight = FontWeight.SemiBold, color = c.ink3, letterSpacing = 0.6.sp)
        if (action != null) TextButton(onClick = onAction) { Text(action, fontSize = Type.small) }
    }
}

/** A list row: dot, title, one line under it, and a short label at the end. One tap opens it. */
@Composable
fun ListRow(title: String, line: String, label: String, tone: Tone, glyph: String, onClick: () -> Unit) {
    val c = LocalLp.current
    Row(
        Modifier.fillMaxWidth().clickable(role = Role.Button, onClick = onClick).heightIn(min = 56.dp).padding(horizontal = 16.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Dot(toneColor(tone))
        Column(Modifier.weight(1f)) {
            Text("$glyph $title", fontSize = Type.body, fontWeight = FontWeight.Medium, color = c.ink, maxLines = 2, overflow = TextOverflow.Ellipsis)
            if (line.isNotBlank()) Text(line, fontSize = Type.small, color = c.ink3, maxLines = 2, overflow = TextOverflow.Ellipsis)
        }
        if (label.isNotBlank()) Text(label, fontSize = Type.tiny, color = if (tone == Tone.Idle) c.ink3 else toneColor(tone), maxLines = 1)
    }
}

@Composable
fun Card(modifier: Modifier = Modifier, border: Color = LocalLp.current.line2, content: @Composable () -> Unit) {
    val c = LocalLp.current
    Box(modifier.clip(RoundedCornerShape(10.dp)).background(c.paper).border(1.dp, border, RoundedCornerShape(10.dp)).padding(12.dp)) { content() }
}

/** A bar under the top bar: a state that is not Live, with its one button. */
@Composable
fun Banner(text: String, action: String? = null, onAction: () -> Unit = {}) {
    val c = LocalLp.current
    Row(Modifier.fillMaxWidth().background(c.attnSoft).padding(start = 16.dp, end = 4.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(text, Modifier.weight(1f).padding(vertical = 10.dp), fontSize = Type.small, color = c.attn)
        if (action != null) TextButton(onClick = onAction) { Text(action, fontSize = Type.small, color = c.attn, fontWeight = FontWeight.SemiBold) } else Spacer(Modifier.width(12.dp))
    }
}

@Composable
fun Muted(text: String, modifier: Modifier = Modifier) {
    Text(text, modifier, fontSize = Type.small, color = MaterialTheme.colorScheme.onSurfaceVariant)
}

/** Short relative times, never seconds (the web's `ago`). */
fun ago(ms: Double?, now: Long = System.currentTimeMillis()): String {
    if (ms == null || ms <= 0) return ""
    val min = ((now - ms) / 60_000).toLong()
    return when {
        min < 1 -> "now"
        min < 60 -> "$min min ago"
        min < 60 * 24 -> "${min / 60} h ago"
        else -> "${min / (60 * 24)} d ago"
    }
}

/** How long something has lasted: "12 min", "3 h". */
fun since(ms: Double, now: Long = System.currentTimeMillis()): String {
    val min = ((now - ms) / 60_000).toLong().coerceAtLeast(0)
    return when {
        min < 1 -> "just started"
        min < 60 -> "$min min"
        min < 60 * 24 -> "${min / 60} h ${min % 60} min"
        else -> "${min / (60 * 24)} d"
    }
}
