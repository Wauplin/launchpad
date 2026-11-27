package dev.launchpad.android.ui

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.util.LruCache
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.withLink
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import dev.launchpad.android.Graph
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.commonmark.ext.autolink.AutolinkExtension
import org.commonmark.ext.gfm.strikethrough.Strikethrough
import org.commonmark.ext.gfm.strikethrough.StrikethroughExtension
import org.commonmark.ext.gfm.tables.TableBlock
import org.commonmark.ext.gfm.tables.TableCell
import org.commonmark.ext.gfm.tables.TableRow
import org.commonmark.ext.gfm.tables.TablesExtension
import org.commonmark.node.BlockQuote
import org.commonmark.node.BulletList
import org.commonmark.node.Code
import org.commonmark.node.Emphasis
import org.commonmark.node.FencedCodeBlock
import org.commonmark.node.HardLineBreak
import org.commonmark.node.Heading
import org.commonmark.node.HtmlBlock
import org.commonmark.node.HtmlInline
import org.commonmark.node.Image as MdImage
import org.commonmark.node.IndentedCodeBlock
import org.commonmark.node.Link
import org.commonmark.node.ListItem
import org.commonmark.node.Node
import org.commonmark.node.OrderedList
import org.commonmark.node.Paragraph
import org.commonmark.node.SoftLineBreak
import org.commonmark.node.StrongEmphasis
import org.commonmark.node.Text as MdText
import org.commonmark.node.ThematicBreak
import org.commonmark.parser.Parser

// Markdown of chats and traces, drawn natively (CommonMark with GitHub tables, strikethrough and autolinks):
// paragraphs, headings, lists, quotes, code blocks (monospace, scrolling sideways), tables (scrolling sideways), links
// (Launchpad links open their screen, others the browser) and images loaded with the device headers. HTML is shown
// as text, never interpreted.

private val parser: Parser = Parser.builder().extensions(listOf(TablesExtension.create(), StrikethroughExtension.create(), AutolinkExtension.create())).build()

private fun Node.children(): List<Node> {
    val out = ArrayList<Node>()
    var n = firstChild
    while (n != null) { out += n; n = n.next }
    return out
}

@Composable
fun Markdown(text: String, modifier: Modifier = Modifier) {
    val doc = remember(text) { parser.parse(text) }
    SelectionContainer {
        Column(modifier, verticalArrangement = Arrangement.spacedBy(6.dp)) {
            for (block in doc.children()) Block(block)
        }
    }
}

@Composable
private fun Block(node: Node, depth: Int = 0) {
    val c = LocalLp.current
    when (node) {
        is Paragraph -> {
            val image = node.children().singleOrNull() as? MdImage
            if (image != null) RemoteImage(image.destination, image.title ?: "") else InlineText(node)
        }
        is Heading -> InlineText(node, size = if (node.level <= 2) Type.title else Type.body, weight = FontWeight.SemiBold)
        is BulletList, is OrderedList -> {
            var n = (node as? OrderedList)?.markerStartNumber ?: 1
            Column(Modifier.padding(start = 4.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                for (item in node.children()) {
                    val marker = if (node is OrderedList) "${n++}." else "•"
                    Row {
                        Text(marker, Modifier.width(22.dp), fontSize = Type.body, color = c.ink3)
                        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                            for (child in (item as? ListItem)?.children() ?: emptyList()) Block(child, depth + 1)
                        }
                    }
                }
            }
        }
        is BlockQuote -> Row {
            Box(Modifier.width(3.dp).heightIn(min = 20.dp).background(c.line))
            Column(Modifier.padding(start = 10.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) { for (child in node.children()) Block(child, depth + 1) }
        }
        is FencedCodeBlock -> CodeBlock(node.literal.trimEnd('\n'))
        is IndentedCodeBlock -> CodeBlock(node.literal.trimEnd('\n'))
        is HtmlBlock -> CodeBlock(node.literal.trimEnd('\n'))
        is ThematicBreak -> Box(Modifier.fillMaxWidth().padding(vertical = 4.dp).heightIn(min = 1.dp).background(c.line2))
        is TableBlock -> Table(node)
        else -> node.children().forEach { Block(it, depth) }
    }
}

@Composable
fun CodeBlock(code: String) {
    val c = LocalLp.current
    Box(Modifier.fillMaxWidth().clip(RoundedCornerShape(8.dp)).background(c.sunk).horizontalScroll(rememberScrollState()).padding(10.dp)) {
        Text(code, fontFamily = FontFamily.Monospace, fontSize = Type.small, color = c.ink, softWrap = false)
    }
}

@Composable
private fun Table(table: TableBlock) {
    val c = LocalLp.current
    val rows = ArrayList<Pair<List<Node>, Boolean>>()
    for (section in table.children()) for (row in section.children()) if (row is TableRow) rows += row.children() to (row.children().firstOrNull() as? TableCell)?.isHeader.let { it == true }
    Box(Modifier.horizontalScroll(rememberScrollState()).border(1.dp, c.line2, RoundedCornerShape(6.dp))) {
        Column {
            for ((cells, header) in rows) Row(Modifier.background(if (header) c.sunk else c.paper)) {
                for (cell in cells) Box(Modifier.widthIn(min = 72.dp, max = 260.dp).padding(horizontal = 8.dp, vertical = 6.dp)) {
                    InlineText(cell, size = Type.small, weight = if (header) FontWeight.SemiBold else FontWeight.Normal)
                }
            }
        }
    }
}

@Composable
private fun InlineText(node: Node, size: androidx.compose.ui.unit.TextUnit = Type.body, weight: FontWeight = FontWeight.Normal) {
    val c = LocalLp.current
    val context = LocalContext.current
    val styles = TextLinkStyles(SpanStyle(color = c.tealInk, textDecoration = TextDecoration.Underline))
    val text = remember(node, c) {
        buildAnnotatedString {
            fun walk(n: Node) {
                when (n) {
                    is MdText -> append(n.literal)
                    is Code -> withStyle(SpanStyle(fontFamily = FontFamily.Monospace, background = c.sunk)) { append(n.literal) }
                    is Emphasis -> withStyle(SpanStyle(fontStyle = FontStyle.Italic)) { n.children().forEach(::walk) }
                    is StrongEmphasis -> withStyle(SpanStyle(fontWeight = FontWeight.SemiBold)) { n.children().forEach(::walk) }
                    is Strikethrough -> withStyle(SpanStyle(textDecoration = TextDecoration.LineThrough)) { n.children().forEach(::walk) }
                    is Link -> {
                        val url = n.destination
                        withLink(LinkAnnotation.Clickable(url, styles) { Links.open(context, url) }) { n.children().forEach(::walk) }
                    }
                    is MdImage -> append("[image: ${n.title ?: n.destination}]")
                    is SoftLineBreak -> append(" ")
                    is HardLineBreak -> append("\n")
                    is HtmlInline -> append(n.literal)
                    else -> n.children().forEach(::walk)
                }
            }
            node.children().forEach(::walk)
        }
    }
    Text(text, fontSize = size, fontWeight = weight, color = c.ink)
}

/** Plain text with its links made tappable (messages that are not Markdown: channel posts, the owner's own text). */
@Composable
fun LinkedText(text: String, color: androidx.compose.ui.graphics.Color = LocalLp.current.ink) {
    val c = LocalLp.current
    val context = LocalContext.current
    val styles = TextLinkStyles(SpanStyle(color = c.tealInk, textDecoration = TextDecoration.Underline))
    val annotated = remember(text, c) {
        buildAnnotatedString {
            var at = 0
            for (m in URL.findAll(text)) {
                append(text.substring(at, m.range.first))
                val url = m.value.trimEnd('.', ',', ')', ';', ':')
                withLink(LinkAnnotation.Clickable(url, styles) { Links.open(context, url) }) { append(url) }
                append(m.value.substring(url.length))
                at = m.range.last + 1
            }
            append(text.substring(at))
        }
    }
    SelectionContainer { Text(annotated, fontSize = Type.body, color = color) }
}

private val URL = Regex("""(https?://\S+|lp:[A-Za-z_]+/\S+)""")

// ---------------------------------------------------------------- images through the Home client

private val images = LruCache<String, Bitmap>(24)

/**
 * An image of a message: a data URL, Home's upload reference (`upload:<type>;<sha256>`) or a Home path, read with the
 * device headers (the page never has them); other addresses show as a link.
 */
@Composable
fun RemoteImage(source: String, description: String) {
    val c = LocalLp.current
    var bitmap by remember(source) { mutableStateOf(images.get(source)) }
    var failed by remember(source) { mutableStateOf(false) }
    LaunchedEffect(source) {
        if (bitmap != null) return@LaunchedEffect
        val loaded = withContext(Dispatchers.IO) { runCatching { load(source) }.getOrNull() }
        if (loaded != null) { images.put(source, loaded); bitmap = loaded } else failed = true
    }
    val b = bitmap
    when {
        b != null -> Image(b.asImageBitmap(), description, Modifier.widthIn(max = 320.dp).heightIn(max = 320.dp).clip(RoundedCornerShape(8.dp)), contentScale = ContentScale.Fit)
        failed -> Muted("🖼 ${description.ifBlank { "Image" }}")
        else -> Box(Modifier.width(120.dp).heightIn(min = 80.dp).clip(RoundedCornerShape(8.dp)).background(c.sunk))
    }
}

private suspend fun load(source: String): Bitmap? {
    val bytes: ByteArray = when {
        source.startsWith("data:") -> android.util.Base64.decode(source.substringAfter(","), android.util.Base64.DEFAULT)
        source.startsWith("upload:") -> {
            val m = Regex("""^upload:(image/[a-z0-9.+-]+);([0-9a-f]{64})$""").find(source) ?: return null
            Graph.home.raw("GET", "/api/uploads/${m.groupValues[2]}?type=${java.net.URLEncoder.encode(m.groupValues[1], "UTF-8")}").takeIf { it.status < 400 }?.bytes ?: return null
        }
        source.startsWith("/api/") -> Graph.home.raw("GET", source).takeIf { it.status < 400 }?.bytes ?: return null
        else -> return null
    }
    return BitmapFactory.decodeByteArray(bytes, 0, bytes.size)
}
