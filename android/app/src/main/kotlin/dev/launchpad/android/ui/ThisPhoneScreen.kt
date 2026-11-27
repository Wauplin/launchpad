package dev.launchpad.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.FilterChip
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.launchpad.android.BuildConfig
import dev.launchpad.android.Graph
import kotlinx.coroutines.launch

/**
 * This phone: the only settings the app has (the rest is ↗ in the browser). Later milestones add their rows here
 * (notifications, updates) as [Field]s or sections.
 */
@Composable
fun ThisPhoneScreen() {
    val c = LocalLp.current
    val context = LocalContext.current
    val link by Graph.store.link.collectAsStateWithLifecycle()
    val settings by Graph.store.settings.collectAsStateWithLifecycle()
    var confirm by remember { mutableStateOf(false) }
    Column(Modifier.fillMaxSize().background(c.paper)) {
        ScreenBar("This phone", onBack = { Graph.nav.back() })
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).navigationBarsPadding().padding(16.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
            Field("Phone", link.deviceName)
            Field("Home", "${link.space}\n${link.origin}")
            if (link.account.isNotEmpty()) Field("Hugging Face account", link.account)
            Field("App", "Launchpad ${BuildConfig.VERSION_NAME} (${BuildConfig.CHANNEL}${if (BuildConfig.COMMIT.isNotEmpty()) " · " + BuildConfig.COMMIT.take(7) else ""})")
            Column {
                Text("Theme", fontSize = Type.small, color = c.ink3)
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    for ((value, label) in listOf("system" to "System", "light" to "Light", "dark" to "Dark")) {
                        FilterChip(selected = settings.theme == value, onClick = { Graph.scope.launch { Graph.store.setSettings(Graph.store.settings.value.copy(theme = value)) } }, label = { Text(label) })
                    }
                }
            }
            OutlinedButton(onClick = { openInBrowser(context, webPath(Route.ThisPhone, Graph.repo.workspaceId.value)) }, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) { Text("Settings in the browser ↗") }
            OutlinedButton(onClick = { confirm = true }, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) { Text("Sign out", color = c.danger) }
            Muted("Sign out deletes this phone from Home's devices, then its secrets and cached data from the phone.")
        }
    }
    if (confirm) {
        AlertDialog(
            onDismissRequest = { confirm = false },
            title = { Text("Sign out of Launchpad?") },
            text = { Text("Home forgets this phone. Connecting again needs Sign in with Hugging Face.") },
            confirmButton = { TextButton(onClick = { confirm = false; Graph.scope.launch { Graph.signOut() } }) { Text("Sign out") } },
            dismissButton = { TextButton(onClick = { confirm = false }) { Text("Cancel") } },
        )
    }
}

@Composable
fun Field(label: String, value: String) {
    val c = LocalLp.current
    Column {
        Text(label, fontSize = Type.small, color = c.ink3)
        Text(value, fontSize = Type.body, color = c.ink, fontWeight = FontWeight.Medium)
    }
}
