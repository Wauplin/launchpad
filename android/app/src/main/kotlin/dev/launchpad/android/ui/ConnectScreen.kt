package dev.launchpad.android.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.systemBarsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.launchpad.android.Graph
import dev.launchpad.android.SignInStep
import dev.launchpad.android.data.Tone

/** Connect: Sign in with Hugging Face is the only way in (no pairing link, QR code or token). */
@Composable
fun ConnectScreen() {
    val c = LocalLp.current
    val context = LocalContext.current
    val step by Graph.signIn.step.collectAsStateWithLifecycle()
    val notice by Graph.notice.collectAsStateWithLifecycle()
    BackHandler(enabled = step !is SignInStep.Start) { Graph.signIn.cancel() }
    Column(
        Modifier.fillMaxSize().background(c.paper).systemBarsPadding().imePadding().verticalScroll(rememberScrollState()).padding(20.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        when (val s = step) {
            is SignInStep.Start -> {
                Title("Launchpad")
                Muted("This phone becomes a client of your Home on Hugging Face, like the Chrome extension and the desktop companion.")
                notice?.let { Problem(it) }
                s.error?.let { Problem(it) }
                BigButton("Sign in with Hugging Face") { Graph.signIn.start(context) }
                Muted("One way in. No pairing code, QR code or token.")
            }
            SignInStep.Browser -> {
                Title("Sign in with Hugging Face")
                Muted("Approve Launchpad on huggingface.co. The page returns here by itself.")
                Waiting("Waiting for Hugging Face…")
                OutlinedButton(onClick = { Graph.signIn.start(context) }, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) { Text("Open Hugging Face again") }
                OutlinedButton(onClick = { Graph.signIn.cancel() }, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) { Text("Cancel") }
            }
            SignInStep.Finding -> {
                Title("Finding your Home")
                Waiting("Looking for Launchpad Spaces in your account and organizations…")
            }
            is SignInStep.Pick -> {
                Title("Choose your Home")
                if (s.homes.isEmpty()) {
                    Problem("No Launchpad Space found in the account ${s.account} and its organizations.")
                    Muted("Home's Space carries the tag launchpad-home. Deploy Home first, or tag your Space, then look again.")
                } else {
                    Muted("Launchpad Spaces of ${s.account} and its organizations.")
                    for (home in s.homes) {
                        val state = when {
                            home.answers -> "running"
                            home.stage == "RUNNING" -> if (home.own) "running, did not answer" else "running (organization)"
                            else -> (home.stage ?: "unknown").lowercase().replace('_', ' ')
                        }
                        Row(
                            Modifier.fillMaxWidth().clip(RoundedCornerShape(10.dp)).background(c.chrome).clickable(role = Role.Button) { Graph.signIn.pick(home) }.heightIn(min = 56.dp).padding(12.dp),
                            verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp),
                        ) {
                            Dot(toneColor(if (home.answers) Tone.Done else Tone.Idle))
                            Column(Modifier.weight(1f)) {
                                Text(home.space, fontSize = Type.body, fontWeight = FontWeight.Medium, color = c.ink)
                                Text("private · $state", fontSize = Type.small, color = c.ink3)
                            }
                            Text("›", fontSize = Type.big, color = c.ink3)
                        }
                    }
                }
                OutlinedButton(onClick = { Graph.signIn.findAgain() }, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) { Text("Look again") }
                OutlinedButton(onClick = { Graph.signIn.cancel() }, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) { Text("Cancel") }
            }
            is SignInStep.Name -> {
                var name by remember { mutableStateOf(Graph.signIn.defaultName) }
                Title("Name this phone")
                Muted("Home: ${s.home.space}")
                OutlinedTextField(value = name, onValueChange = { name = it.take(80) }, singleLine = true, label = { Text("Phone name") }, modifier = Modifier.fillMaxWidth())
                Muted("Home's Space checks your Hugging Face login and creates a device for this phone. It shows in Settings → Devices, where Revoke signs it out.")
                s.error?.let { Problem(it) }
                BigButton("Connect") { Graph.signIn.connect(name) }
                OutlinedButton(onClick = { Graph.signIn.cancel() }, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) { Text("Cancel") }
            }
            is SignInStep.Connecting -> {
                Title("Connecting")
                Waiting("Creating “${s.name}” on ${s.home.space}…")
            }
        }
    }
}

@Composable
private fun Title(text: String) {
    Text(text, Modifier.padding(top = 24.dp).semantics { heading() }, fontSize = Type.big, fontWeight = FontWeight.SemiBold, color = LocalLp.current.ink)
}

@Composable
private fun BigButton(text: String, onClick: () -> Unit) {
    Button(onClick = onClick, modifier = Modifier.fillMaxWidth().heightIn(min = 52.dp)) { Text(text, fontSize = Type.body, fontWeight = FontWeight.SemiBold) }
}

@Composable
private fun Problem(text: String) {
    val c = LocalLp.current
    Text(text, Modifier.fillMaxWidth().clip(RoundedCornerShape(8.dp)).background(c.attnSoft).padding(12.dp), fontSize = Type.small, color = c.attn)
}

@Composable
private fun Waiting(text: String) {
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        CircularProgressIndicator(Modifier.padding(4.dp), strokeWidth = 2.dp)
        Muted(text, Modifier.weight(1f))
    }
}
