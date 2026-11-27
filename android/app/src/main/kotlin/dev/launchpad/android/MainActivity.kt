package dev.launchpad.android

import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.input.pointer.pointerInput
import kotlinx.coroutines.launch
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.launchpad.android.ui.ConnectScreen
import dev.launchpad.android.ui.HandoffScreen
import dev.launchpad.android.ui.LaunchpadTheme
import dev.launchpad.android.ui.NowScreen
import dev.launchpad.android.ui.Route
import dev.launchpad.android.ui.Scope
import dev.launchpad.android.ui.ThisPhoneScreen

/** The one activity. The Custom Tab of the sign-in returns here (`dev.launchpad.android://signed-in`, which carries nothing). */
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        Graph.init(application)
        dev.launchpad.android.data.Presence.start()
        // A link or a share that started the app (once: not again after a configuration change).
        if (savedInstanceState == null) dev.launchpad.android.ui.Links.handle(this, intent)
        setContent {
            val settings by Graph.store.settings.collectAsStateWithLifecycle()
            LaunchpadTheme(settings.theme) { AppRoot() }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        dev.launchpad.android.ui.Links.handle(this, intent)
    }
}

/** Connect when this phone is not a device of a Home; else the back stack over Now. */
@Composable
fun AppRoot() {
    val linked by Graph.linked.collectAsStateWithLifecycle()
    val snackbar = remember { SnackbarHostState() }
    LaunchedEffect(Unit) { Graph.toasts.collect { snackbar.showSnackbar(it) } }
    LaunchedEffect(Unit) {
        dev.launchpad.android.ui.Snacks.undo.collect { u ->
            if (snackbar.showSnackbar(u.message, actionLabel = "Undo", duration = androidx.compose.material3.SnackbarDuration.Short) == androidx.compose.material3.SnackbarResult.ActionPerformed) Graph.scope.launch { u.undo() }
        }
    }
    // Every touch counts as the owner using the phone (presence and "seen").
    Box(Modifier.fillMaxSize().pointerInput(Unit) {
        awaitPointerEventScope { while (true) { awaitPointerEvent(androidx.compose.ui.input.pointer.PointerEventPass.Initial); dev.launchpad.android.data.Presence.input() } }
    }) {
        if (!linked) {
            ConnectScreen()
        } else {
            val nav = Graph.nav
            val context = LocalContext.current
            // Back: the top screen, then Now from a scope list, then out of the app (the system's default).
            BackHandler(enabled = nav.stack.isNotEmpty() || nav.scope != Scope.Now) { nav.back() }
            when (val route = nav.top) {
                null -> NowScreen(onAsk = { target, text, files -> dev.launchpad.android.ui.ask(context, target, text, files) })
                is Route.Conversation -> dev.launchpad.android.ui.ConversationScreen(route)
                is Route.ChannelChat -> dev.launchpad.android.ui.ChannelScreen(route)
                is Route.Agent -> dev.launchpad.android.ui.AgentScreen(route)
                Route.Search -> dev.launchpad.android.ui.SearchScreen()
                Route.ThisPhone -> ThisPhoneScreen()
                is Route.FolioReader -> dev.launchpad.android.ui.FolioScreen(route)
                else -> HandoffScreen(route)
            }
        }
        SnackbarHost(snackbar, Modifier.align(Alignment.BottomCenter).navigationBarsPadding().imePadding().padding(bottom = 88.dp))
    }
}
