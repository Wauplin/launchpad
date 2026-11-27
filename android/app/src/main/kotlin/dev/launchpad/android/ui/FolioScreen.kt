package dev.launchpad.android.ui

import android.annotation.SuppressLint
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.view.ViewGroup
import android.webkit.ConsoleMessage
import android.webkit.CookieManager
import android.webkit.JavascriptInterface
import android.webkit.WebChromeClient
import android.webkit.WebResourceError
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material3.Button
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature
import dev.launchpad.android.Graph
import dev.launchpad.android.data.FolioCache
import dev.launchpad.android.data.FolioPosition
import dev.launchpad.android.data.LpJson
import dev.launchpad.android.data.str
import dev.launchpad.android.net.Conn
import dev.launchpad.android.net.HomeException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.doubleOrNull
import org.json.JSONObject
import java.io.ByteArrayInputStream

private const val TAG = "LaunchpadFolio"

/**
 * The writes the reader's page may send through the bridge, as (method, pattern of what follows
 * `/api/folios/<ws>/<slug>/`): marks and comments, Send, Ask, the render report and sketch data of this folio. Nothing
 * else of Home's API is reachable from the page: not another folio, not a session, and none of the folio's own
 * heavier actions (write, restore, share, archive, delete), which stay in the browser.
 */
private val WRITES = listOf(
    "POST" to Regex("marks"),
    "POST" to Regex("marks/[A-Za-z0-9_-]+/(reply|resolve)"),
    "DELETE" to Regex("marks/[A-Za-z0-9_-]+"),
    "POST" to Regex("send|ask|ask-fix|render"),
    "PUT" to Regex("data/[^?#]+"),
    "DELETE" to Regex("data/[^?#]+"),
)

/**
 * The script that gives the page `window.LaunchpadNative` (the contract of web/src/lib/api.ts) on WebViews that can
 * limit a native object to one origin: it only forwards strings to the port, which exists in Home's own frames and
 * not in the sandboxed frames of sketches (their origin is opaque). It holds no secret.
 */
private const val SHIM = """(function () {
  if (window.top !== window || window.LaunchpadNative || !window.LaunchpadNativePort) return;
  var port = window.LaunchpadNativePort;
  window.LaunchpadNative = {
    request: function (id, method, path, body) { port.postMessage(JSON.stringify({ k: "request", id: String(id), method: String(method), path: String(path), body: body == null ? "" : String(body) })); },
    event: function (json) { port.postMessage(JSON.stringify({ k: "event", json: String(json) })); }
  };
})();"""

/** One open folio: what its page reported, and the WebView that shows Home's own reader (`/folio-embed/`). */
private class FolioModel(val ws: String, val slug: String, fallbackTitle: String) {
    val origin: String = Graph.home.origin
    private val base = "/api/folios/${Uri.encode(ws)}/${Uri.encode(slug)}"
    private val main = Handler(Looper.getMainLooper())

    var title by mutableStateOf(fallbackTitle)
    var rev by mutableIntStateOf(0)
    var latestRev by mutableIntStateOf(0)
    var page by mutableStateOf<String?>(null)
    var pages by mutableStateOf<List<Pair<String, String>>>(emptyList())
    var unsent by mutableIntStateOf(0)
    var overlay by mutableIntStateOf(0)
    var ready by mutableStateOf(false)
    var error by mutableStateOf<String?>(null)
    /** Home did not answer: the page shows the saved copy, and its writes fail. */
    var offline by mutableStateOf(false)
    private var position = FolioCache.position(ws, slug)
    var web: WebView? = null

    fun url(dark: Boolean): String {
        val q = StringBuilder("?ws=${Uri.encode(ws)}&slug=${Uri.encode(slug)}")
        position.page?.let { q.append("&page=${Uri.encode(it)}") }
        position.anchor?.let { q.append("&anchor=${Uri.encode(it)}") }
        q.append("&theme=").append(if (dark) "dark" else "light")
        return "$origin/folio-embed/index.html$q"
    }

    /** A command of the contract (`window.__launchpadNative.command(json)`). */
    fun command(type: String, extra: Map<String, String> = emptyMap()) {
        val json = JSONObject(mapOf("type" to type) + extra).toString()
        main.post { web?.evaluateJavascript("window.__launchpadNative&&window.__launchpadNative.command(${JSONObject.quote(json)})", null) }
    }

    private fun resolve(id: String, status: Int, body: String) {
        main.post { web?.evaluateJavascript("window.__launchpadNative&&window.__launchpadNative.resolve(${JSONObject.quote(id)},$status,${JSONObject.quote(body)})", null) }
    }

    // ------------------------------------------------------------ the bridge (page → app)

    private fun writable(method: String, path: String): Boolean {
        val rest = path.removePrefix("$base/")
        // No dot segment can climb out of this folio's routes.
        if (rest.split('/').any { it == ".." || it == "." || it.contains("%2e", true) && it.replace("%2e", ".", true).trim('.').isEmpty() }) return false
        return rest != path && WRITES.any { (m, pattern) -> m == method && pattern.matches(rest) }
    }

    /** A write of the page, run with the app's own client: the page never sees the device secret or the token. */
    fun request(id: String, method: String, path: String, body: String) {
        val verb = method.uppercase()
        if (!writable(verb, path)) {
            Log.w(TAG, "refused $verb $path")
            resolve(id, 403, """{"error":"The Android app does not send this request for a folio"}""")
            return
        }
        Graph.scope.launch(Dispatchers.IO) {
            try {
                val r = Graph.home.raw(verb, path, body.ifEmpty { "{}" }.toByteArray(), "application/json")
                resolve(id, r.status, String(r.bytes))
            } catch (e: HomeException) {
                // Not sent, or no answer: the page shows it as a failed request and repeats nothing by itself.
                offline = true
                resolve(id, 0, "")
            }
        }
    }

    fun event(json: String) {
        val e = runCatching { LpJson.parseToJsonElement(json) as? JsonObject }.getOrNull() ?: return
        fun int(key: String) = (e[key] as? JsonPrimitive)?.doubleOrNull?.toInt()
        when (e.str("type")) {
            "state" -> {
                e.str("title")?.let { title = it }
                int("rev")?.let { rev = it }
                int("latest_rev")?.let { latestRev = it }
                unsent = int("unsent") ?: 0
                page = e.str("page")
                pages = (e["pages"] as? JsonArray)?.mapNotNull { p -> p.str("path")?.let { it to (p.str("title") ?: it) } } ?: pages
                val next = FolioPosition(page, (e["anchor"] as? JsonPrimitive)?.takeIf { it.isString }?.contentOrNull)
                if (next != position && next.page != null) {
                    position = next
                    Graph.scope.launch { FolioCache.setPosition(ws, slug, next) }
                }
            }
            "overlay" -> overlay = int("open") ?: 0
            "ready" -> { ready = true; error = null }
            "error" -> error = e.str("message") ?: "The folio did not load"
            "open" -> e.str("url")?.let { url -> main.post { open(url) } }
        }
    }

    /** The message of the origin-limited port (see [SHIM]). */
    fun message(data: String?) {
        val m = runCatching { LpJson.parseToJsonElement(data ?: "") as? JsonObject }.getOrNull() ?: return
        when (m.str("k")) {
            "request" -> request(m.str("id") ?: return, m.str("method") ?: return, m.str("path") ?: return, m.str("body") ?: "")
            "event" -> event(m.str("json") ?: return)
        }
    }

    /**
     * Fallback for a WebView too old to limit a native object to one origin: `addJavascriptInterface` shows this
     * object to every frame, the sandboxed frames of sketches included, so [writable] is the guard: only the writes
     * of this folio's reader pass.
     */
    val legacy = object {
        @JavascriptInterface
        fun request(id: String?, method: String?, path: String?, body: String?) {
            this@FolioModel.request(id ?: return, method ?: return, path ?: return, body ?: "")
        }

        @JavascriptInterface
        fun event(json: String?) {
            this@FolioModel.event(json ?: return)
        }
    }

    /** A link of the page: another folio opens here, everything else in the browser. */
    fun open(url: String) {
        val uri = runCatching { Uri.parse(url) }.getOrNull() ?: return
        val scheme = uri.scheme?.lowercase()
        if (scheme != "https" && scheme != "http" && scheme != "mailto") return
        val segments = uri.pathSegments
        if (url.startsWith("$origin/") && segments.size == 2 && segments[0] == "folios") {
            Graph.nav.open(Route.FolioReader(segments[1]))
            return
        }
        runCatching { Graph.context.startActivity(Intent(Intent.ACTION_VIEW, uri).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
            .onFailure { Graph.toast("No app to open this link") }
    }

    // ------------------------------------------------------------ reads (the WebView's GETs)

    /**
     * What the page may read: its own files, this folio, and the three lists its reader uses; other GETs of Home's
     * API pass too (the reader's hooks), but never for a frame of another origin (a sketch's frame sends
     * `Origin: null` on what it could read).
     */
    private fun readable(path: String, headers: Map<String, String>): Boolean {
        val from = headers.entries.firstOrNull { it.key.equals("Origin", true) }?.value
        if (from != null && from != origin) return false
        return path.startsWith("/folio-embed/") || path.startsWith("/api/")
    }

    private fun answer(status: Int, contentType: String?, bytes: ByteArray): WebResourceResponse {
        val mime = contentType?.substringBefore(';')?.trim()?.ifEmpty { null } ?: "application/octet-stream"
        val charset = contentType?.substringAfter("charset=", "")?.substringBefore(';')?.trim()?.ifEmpty { null }
            ?: if (mime.startsWith("text/") || mime.endsWith("json") || mime.endsWith("javascript")) "utf-8" else null
        // A WebView answer cannot be a redirect.
        val code = if (status in 300..399 || status !in 100..599) 502 else status
        return WebResourceResponse(mime, charset, code, if (code < 400) "OK" else "Error", mapOf("Cache-Control" to "no-store"), ByteArrayInputStream(bytes))
    }

    private fun refused(status: Int) = answer(status, "application/json", """{"error":"Not available in the folio reader"}""".toByteArray())

    /**
     * Every request of the WebView. A GET to Home is made by the app's client, which adds the device secret and the
     * Hugging Face token, and kept for offline reading; the answer goes back without them. Any other address is
     * refused: the reader needs none.
     */
    fun intercept(request: WebResourceRequest): WebResourceResponse? {
        val uri = request.url
        val scheme = uri.scheme?.lowercase()
        // data:, blob: and about: stay inside the page.
        if (scheme != "https" && scheme != "http") return null
        val port = if (uri.port > 0) ":${uri.port}" else ""
        if ("$scheme://${uri.host}$port" != origin) return refused(403)
        // Writes go through the bridge, never from the page itself.
        if (request.method != "GET" && request.method != "HEAD") return refused(403)
        val path = uri.encodedPath ?: "/"
        if (!readable(path, request.requestHeaders)) return refused(404)
        val full = path + (uri.encodedQuery?.let { "?$it" } ?: "")
        // Known offline: the saved copy first, so the page does not wait for a timeout on every file.
        if (offline) FolioCache.get(ws, slug, full)?.let { return answer(200, it.contentType, it.bytes) }
        return try {
            val r = runBlocking { Graph.home.raw("GET", full) }
            if (r.status == 200) FolioCache.put(ws, slug, full, r.contentType, r.bytes)
            answer(r.status, r.contentType, r.bytes)
        } catch (e: HomeException) {
            offline = true
            FolioCache.get(ws, slug, full)?.let { answer(200, it.contentType, it.bytes) }
                ?: answer(504, "application/json", """{"error":"Offline: this part of the folio was not saved on the phone"}""".toByteArray())
        }
    }

    @SuppressLint("SetJavaScriptEnabled")
    fun create(context: Context, dark: Boolean, background: Int): WebView {
        FolioCache.opened(ws, slug)
        val view = WebView(context)
        view.layoutParams = ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT)
        view.setBackgroundColor(background)
        view.settings.apply {
            javaScriptEnabled = true
            domStorageEnabled = true
            allowFileAccess = false
            allowContentAccess = false
            setSupportMultipleWindows(false)
            javaScriptCanOpenWindowsAutomatically = false
            mixedContentMode = WebSettings.MIXED_CONTENT_NEVER_ALLOW
            // The app answers every request itself and keeps its own copy.
            cacheMode = WebSettings.LOAD_NO_CACHE
            // Android's font size setting applies to the folio too.
            textZoom = (context.resources.configuration.fontScale * 100).toInt()
        }
        CookieManager.getInstance().setAcceptThirdPartyCookies(view, false)
        view.webViewClient = object : WebViewClient() {
            override fun shouldInterceptRequest(v: WebView, request: WebResourceRequest): WebResourceResponse? = intercept(request)

            // The WebView never leaves the reader: a link opens outside.
            override fun shouldOverrideUrlLoading(v: WebView, request: WebResourceRequest): Boolean {
                if (request.hasGesture()) open(request.url.toString())
                return true
            }

            override fun onReceivedError(v: WebView, request: WebResourceRequest, e: WebResourceError) {
                if (request.isForMainFrame) error = "The folio reader did not load (${e.description})"
            }
        }
        view.webChromeClient = object : WebChromeClient() {
            override fun onConsoleMessage(message: ConsoleMessage): Boolean {
                if (message.messageLevel() == ConsoleMessage.MessageLevel.ERROR) Log.w(TAG, "${message.message()} (${message.sourceId()}:${message.lineNumber()})")
                return true
            }
        }
        if (WebViewFeature.isFeatureSupported(WebViewFeature.WEB_MESSAGE_LISTENER) && WebViewFeature.isFeatureSupported(WebViewFeature.DOCUMENT_START_SCRIPT)) {
            // Only Home's own origin gets the port, and only its top frame is listened to.
            WebViewCompat.addWebMessageListener(view, "LaunchpadNativePort", setOf(origin)) { _, message, _, isMainFrame, _ -> if (isMainFrame) message(message.data) }
            WebViewCompat.addDocumentStartJavaScript(view, SHIM, setOf(origin))
        } else {
            view.addJavascriptInterface(legacy, "LaunchpadNative")
        }
        web = view
        // No header and no cookie here: `intercept` adds the credentials outside the WebView.
        view.loadUrl(url(dark))
        return view
    }

    fun reload(dark: Boolean) {
        error = null
        ready = false
        overlay = 0
        web?.loadUrl(url(dark))
    }

    fun close() {
        web?.let {
            it.stopLoading()
            it.destroy()
        }
        web = null
    }
}

/**
 * A folio: the app's chrome (title, rev, page chips, the Send bar with the unsent count) around Home's own reader in a
 * WebView. Marks and comments stay unsent until Send, as on the laptop. It reopens at the page and heading read last,
 * and the last rev opened reads offline.
 */
@Composable
fun FolioScreen(route: Route.FolioReader) {
    val c = LocalLp.current
    val context = LocalContext.current
    val dark = MaterialTheme.colorScheme.background.luminance() < 0.5f
    val folios by Graph.repo.folios.collectAsStateWithLifecycle()
    val conn by Graph.stream.conn.collectAsStateWithLifecycle()
    val model = remember(route.slug) {
        val ws = Graph.repo.workspaceId.value
        FolioModel(ws, route.slug, route.title.ifEmpty { Graph.repo.folios.value.firstOrNull { it.slug == route.slug }?.title ?: route.slug })
    }
    var menu by remember { mutableStateOf(false) }

    DisposableEffect(model) { onDispose { model.close() } }
    // Back closes the page's top sheet (comments, Send) first; then the app's own back leaves the screen.
    BackHandler(enabled = model.overlay > 0) { model.command("back") }
    LaunchedEffect(dark, model.ready) { if (model.ready) model.command("theme", mapOf("theme" to if (dark) "dark" else "light")) }
    // The page has no live stream: the app tells it when a folio changed.
    LaunchedEffect(model) { Graph.stream.events.collect { if (it.topic == "folios" && model.ready) model.command("refresh") } }
    LaunchedEffect(conn) {
        if (conn is Conn.Offline) model.offline = true
        if (conn is Conn.Live && model.offline) {
            model.offline = false
            if (model.ready) model.command("refresh") else model.reload(dark)
        }
    }

    val listed = folios.firstOrNull { it.slug == route.slug }
    val rev = if (model.rev > 0) model.rev else listed?.rev?.toInt() ?: 0
    Column(Modifier.fillMaxSize().background(c.paper).imePadding()) {
        ScreenBar(model.title, onBack = { Graph.nav.back() }) {
            if (rev > 0) Text(if (model.latestRev > rev) "rev $rev of ${model.latestRev}" else "rev $rev", fontSize = Type.tiny, color = c.ink3, modifier = Modifier.padding(horizontal = 4.dp))
            Box {
                GlyphButton("⋯", "More", { menu = true })
                DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                    DropdownMenuItem(text = { Text("Comments") }, enabled = model.ready, onClick = { menu = false; model.command("comments") })
                    DropdownMenuItem(text = { Text("Refresh") }, onClick = { menu = false; model.offline = false; model.reload(dark) })
                    DropdownMenuItem(text = { Text("Open in browser ↗") }, onClick = { menu = false; openInBrowser(context, webPath(route, model.ws)) })
                }
            }
        }
        if (model.pages.size > 1) {
            Row(Modifier.fillMaxWidth().background(c.chrome).horizontalScroll(rememberScrollState()).padding(horizontal = 12.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                for ((path, title) in model.pages) {
                    FilterChip(selected = path == model.page, onClick = { if (path != model.page) model.command("page", mapOf("page" to path)) }, label = { Text(title, fontSize = Type.small, maxLines = 1) }, modifier = Modifier.heightIn(min = 44.dp))
                }
            }
            HorizontalDivider(color = c.line2)
        }
        if (model.offline) Banner("Offline · the copy saved on this phone; marks are not sent", "Retry now") { Graph.stream.retryNow(); model.offline = false; model.reload(dark) }
        Box(Modifier.weight(1f).fillMaxWidth()) {
            AndroidView(factory = { model.web ?: model.create(it, dark, c.paper.toArgb()) }, modifier = Modifier.fillMaxSize())
            val failed = model.error
            if (failed != null) {
                Column(Modifier.fillMaxSize().background(c.paper).padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text(failed, fontSize = Type.body, color = c.danger)
                    Button(onClick = { model.offline = false; model.reload(dark) }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Try again") }
                }
            } else if (!model.ready) {
                Box(Modifier.fillMaxSize().background(c.paper)) { Muted("Loading the folio…", Modifier.padding(16.dp)) }
            }
        }
        if (model.unsent > 0 && model.overlay == 0 && model.error == null) {
            HorizontalDivider(color = c.line2)
            Row(Modifier.fillMaxWidth().background(c.chrome).navigationBarsPadding().padding(horizontal = 16.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(if (model.unsent == 1) "1 unsent mark" else "${model.unsent} unsent marks", Modifier.weight(1f), fontSize = Type.body, color = c.attn, fontWeight = FontWeight.Medium)
                Button(onClick = { model.command("send") }, enabled = !model.offline, modifier = Modifier.heightIn(min = 48.dp)) { Text("Send") }
            }
        }
    }
}
