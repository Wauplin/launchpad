package dev.launchpad.android.ui

import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.launchpad.android.Graph

/** A screen above Now. To add one: a class here, its composable in `AppRoot`'s `when`, and its page in [webPath]. */
sealed interface Route {
    /** A bot's thread. */
    data class Conversation(val botId: String, val thread: String, val title: String = "") : Route
    /** A bot channel or DM. */
    data class ChannelChat(val channelId: String, val title: String = "") : Route
    data class Agent(val sessionId: String, val title: String = "") : Route
    data class FolioReader(val slug: String, val title: String = "") : Route
    /** A Today card the phone does not act on (a proposal, a Local PR…): it opens in the browser. */
    data class Card(val itemId: String, val title: String = "") : Route
    data object ThisPhone : Route
    /** ⌕: the loaded conversations, agents and folios of the workspace, by title. */
    data object Search : Route
}

/** What another app shared into Launchpad: it waits in the Ask box until sent or cleared. */
data class Shared(val text: String, val uris: List<Uri>)

/** What the first screen lists: Now, or one full list (the scope chips). */
enum class Scope(val label: String) { Now("Now"), Chats("Chats"), Agents("Agents"), Folios("Folios") }

/**
 * The back stack (navigation model C: no tab bar). Android back closes the top screen, then returns to Now from a
 * scope list, then leaves the app.
 */
class Navigator {
    val stack = mutableStateListOf<Route>()
    var scope by mutableStateOf(Scope.Now)
    /** Shared text, links and files for the Ask box (Android's share sheet). */
    var shared by mutableStateOf<Shared?>(null)

    fun share(text: String, uris: List<Uri>) {
        shared = Shared(text, uris)
    }

    val top: Route? get() = stack.lastOrNull()

    fun open(route: Route) {
        if (stack.lastOrNull() != route) stack.add(route)
    }

    /** A notification or a link: its screen with Now under it. */
    fun openFromOutside(route: Route) {
        stack.clear()
        scope = Scope.Now
        stack.add(route)
    }

    /** False: nothing left to close, the app leaves. */
    fun back(): Boolean {
        if (stack.isNotEmpty()) {
            stack.removeAt(stack.lastIndex)
            return true
        }
        if (scope != Scope.Now) {
            scope = Scope.Now
            return true
        }
        return false
    }

    fun reset() {
        stack.clear()
        scope = Scope.Now
    }
}

/** The web app's page of a route (web/src/app/router.tsx), for the ↗ handoff. */
fun webPath(route: Route?, workspace: String): String {
    val ws = Uri.encode(workspace)
    return when (route) {
        is Route.Conversation -> "/bots/${Uri.encode(route.botId)}?thread=${Uri.encode(route.thread)}&ws=$ws"
        is Route.ChannelChat -> "/channels/${Uri.encode(route.channelId)}?ws=$ws"
        is Route.Agent -> "/agents?session=${Uri.encode(route.sessionId)}&ws=$ws"
        is Route.FolioReader -> "/folios/${Uri.encode(route.slug)}?ws=$ws"
        is Route.Card -> "/?card=${Uri.encode(route.itemId)}&ws=$ws"
        Route.ThisPhone -> "/settings/devices?ws=$ws"
        Route.Search -> "/chats?ws=$ws"
        null -> "/?ws=$ws"
    }
}

/** ↗: opens Home's page in the browser (signed in there as a browser), for everything the phone does not do. */
fun openInBrowser(context: Context, path: String) {
    val origin = Graph.store.link.value.origin
    if (origin.isEmpty()) return
    runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(origin + path)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
        .onFailure { Graph.toast("No browser to open this page") }
}
