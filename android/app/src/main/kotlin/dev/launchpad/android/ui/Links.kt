package dev.launchpad.android.ui

import android.content.Context
import android.content.Intent
import android.net.Uri
import dev.launchpad.android.Graph
import dev.launchpad.android.data.isBot
import dev.launchpad.android.data.thread
import dev.launchpad.android.gen.LpRef
import dev.launchpad.android.gen.RefTarget
import dev.launchpad.android.net.HomeException
import kotlinx.coroutines.launch
import java.net.URLEncoder

/**
 * Links inside the app: a pasted or tapped `lp:` ID or `https://<home>/…` address opens its native screen when the
 * phone has one (an agent, a bot chat, a channel, a folio), else Home's page in the browser. Home resolves the
 * reference (`GET /api/refs/resolve`, docs/invariants/references.md); other addresses open in the browser.
 */
object Links {
    /** Whether a text is a Launchpad reference the app resolves. */
    fun isLaunchpad(text: String): Boolean {
        val t = text.trim()
        val origin = Graph.store.link.value.origin
        return t.startsWith("lp:") || (origin.isNotEmpty() && (t == origin || t.startsWith("$origin/")))
    }

    /** The native screen of a resolved reference, or null (it opens in the browser). */
    fun routeOf(ref: LpRef, label: String = ""): Route? = when (ref.kind) {
        "session" -> {
            val s = Graph.repo.session(ref.id)
            if (s != null && s.isBot) Route.Conversation(s.launch_spec.bot?.id ?: "", s.thread, label) else Route.Agent(ref.id, label)
        }
        "chat" -> ref.thread?.let { Route.Conversation(ref.id, it, label) }
        "channel" -> Route.ChannelChat(ref.id, label)
        "folio" -> Route.FolioReader(ref.id, label)
        "card" -> Route.Card(ref.id, label)
        else -> null
    }

    /** The screen of a session: its bot's chat at its thread for a bot session, else the agent. */
    fun sessionRoute(sessionId: String, title: String = ""): Route {
        val s = Graph.repo.session(sessionId)
        return if (s != null && s.isBot) Route.Conversation(s.launch_spec.bot?.id ?: "", s.thread, title) else Route.Agent(sessionId, title)
    }

    /**
     * Opens a Today item's screen with Now under it (a notification): its agent or chat, its channel, else its card.
     * `itemId` is Today's id (`session:<id>`, `channel:<id>`, `proposal:<id>`…).
     */
    fun openItem(itemId: String, title: String = "") {
        val (kind, id) = itemId.substringBefore(':') to itemId.substringAfter(':')
        val route = when (kind) {
            "session" -> sessionRoute(id, title)
            "channel" -> Route.ChannelChat(id, title)
            else -> Graph.repo.today.value.items.firstOrNull { it.id == itemId }?.let { routeOf(it, Graph.repo.sessions.value) } ?: Route.Card(itemId, title)
        }
        Graph.nav.openFromOutside(route)
    }

    /** A tap on a link of a message: native when it can, else the browser. */
    fun open(context: Context, url: String) {
        if (!isLaunchpad(url)) {
            runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
                .onFailure { Graph.toast("No app to open this link") }
            return
        }
        Graph.scope.launch { openResolved(context, url, fromOutside = false) }
    }

    /** Resolves a Launchpad link and opens it (`fromOutside`: a shared or opened link, with Now under it). */
    suspend fun openResolved(context: Context, text: String, fromOutside: Boolean) {
        val t = text.trim()
        val target = try {
            Graph.home.get("/api/refs/resolve?ref=" + URLEncoder.encode(t, "UTF-8"), RefTarget.serializer())
        } catch (e: HomeException) {
            // Not a reference Home knows: a Home page still opens in the browser.
            val origin = Graph.store.link.value.origin
            if (t.startsWith(origin)) openInBrowser(context, t.removePrefix(origin).ifEmpty { "/" }) else Graph.toast(e.message ?: "Unknown link")
            return
        }
        if (target.workspace_id.isNotEmpty() && target.workspace_id != Graph.repo.workspaceId.value) Graph.repo.switchWorkspace(target.workspace_id)
        val route = routeOf(target.ref, target.label)
        when {
            route == null -> openInBrowser(context, target.path.ifEmpty { "/" })
            fromOutside -> Graph.nav.openFromOutside(route)
            else -> Graph.nav.open(route)
        }
    }

    /**
     * An intent that reached the activity: a link shared from another app or opened in the app goes to its screen when
     * it is a Launchpad one; other shared text, links and images go to the Ask box.
     */
    fun handle(context: Context, intent: Intent?) {
        if (intent == null || !Graph.linked.value) return
        // A notification: its screen with Now under it (empty: Now itself).
        intent.getStringExtra(dev.launchpad.android.push.EXTRA_OPEN)?.let { target ->
            intent.removeExtra(dev.launchpad.android.push.EXTRA_OPEN)
            val parts = target.split(":", limit = 3)
            when (parts[0]) {
                "session" -> Graph.nav.openFromOutside(sessionRoute(parts.getOrElse(1) { "" }))
                "thread" -> if (parts.size == 3) Graph.nav.openFromOutside(Route.Conversation(parts[1], parts[2]))
                "channel" -> Graph.nav.openFromOutside(Route.ChannelChat(parts.getOrElse(1) { "" }))
                "card" -> openItem(target.removePrefix("card:"))
                else -> Graph.nav.reset()
            }
            return
        }
        when (intent.action) {
            Intent.ACTION_SEND, Intent.ACTION_SEND_MULTIPLE -> {
                val text = listOfNotNull(intent.getStringExtra(Intent.EXTRA_SUBJECT), intent.getStringExtra(Intent.EXTRA_TEXT)).filter { it.isNotBlank() }.distinct().joinToString("\n")
                @Suppress("DEPRECATION")
                val uris: List<Uri> = when (intent.action) {
                    Intent.ACTION_SEND -> listOfNotNull(intent.getParcelableExtra(Intent.EXTRA_STREAM) as? Uri)
                    else -> intent.getParcelableArrayListExtra<Uri>(Intent.EXTRA_STREAM) ?: emptyList()
                }
                Graph.nav.reset()
                Graph.nav.share(text, uris)
            }
            Intent.ACTION_VIEW -> {
                val data = intent.data?.toString() ?: return
                if (data.startsWith("dev.launchpad.android://")) return
                Graph.scope.launch { openResolved(context, data, fromOutside = true) }
            }
        }
        // Handled once: a configuration change must not open it again.
        intent.action = null
    }
}
