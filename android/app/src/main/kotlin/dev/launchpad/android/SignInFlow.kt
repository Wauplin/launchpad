package dev.launchpad.android

import android.content.Context
import android.os.Build
import androidx.browser.customtabs.CustomTabsIntent
import dev.launchpad.android.data.HomeLink
import dev.launchpad.android.data.str
import dev.launchpad.android.net.FoundHome
import dev.launchpad.android.net.HfAuth
import dev.launchpad.android.net.HfRefused
import dev.launchpad.android.net.HfTransient
import dev.launchpad.android.net.HomeException
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch

/** Where Connect is (signin.md, steps L-1 to L-3). */
sealed interface SignInStep {
    /** The one button, with the last failure when there is one. */
    data class Start(val error: String? = null) : SignInStep
    /** huggingface.co is open in a Custom Tab. */
    data object Browser : SignInStep
    /** Looking for the account's Launchpad Spaces. */
    data object Finding : SignInStep
    /** Several Spaces (or none: `homes` empty), with the account's name. */
    data class Pick(val account: String, val homes: List<FoundHome>, val error: String? = null) : SignInStep
    data class Name(val account: String, val home: FoundHome, val error: String? = null) : SignInStep
    data class Connecting(val account: String, val home: FoundHome, val name: String) : SignInStep
}

/**
 * Sign in with Hugging Face, the only way to connect: the Custom Tab, the choice of the Space, the phone's name, and
 * the gateway's `POST /_home/phone/connect`. `reauth` reruns only the Hugging Face step and keeps the device.
 */
class SignInFlow {
    private val _step = MutableStateFlow<SignInStep>(SignInStep.Start())
    val step: StateFlow<SignInStep> = _step
    private var job: Job? = null
    private var pending: HfAuth.Pending? = null

    val defaultName: String get() = Build.MODEL?.takeIf { it.isNotBlank() }?.take(80) ?: "Android phone"

    /** `context`: the activity (the Custom Tab opens in its task). */
    fun start(context: Context, reauth: Boolean = false) {
        cancel()
        if (Graph.hf.clientId.isEmpty()) {
            _step.value = SignInStep.Start("This build has no Hugging Face client ID, so it cannot sign in. Install the APK from Launchpad's GitHub releases.")
            return
        }
        job = Graph.scope.launch {
            try {
                val p = Graph.hf.begin().also { pending = it }
                _step.value = SignInStep.Browser
                CustomTabsIntent.Builder().setShowTitle(true).build().launchUrl(context, p.authorizeUrl)
                Graph.hf.await(p) { Graph.bringToFront() }
                pending = null
                if (reauth) {
                    _step.value = SignInStep.Start()
                    Graph.stream.retryNow()
                    Graph.scope.launch { Graph.repo.refresh() }
                    return@launch
                }
                find()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                Graph.bringToFront()
                _step.value = SignInStep.Start(message(e))
                // Sign in again runs over Now, where Connect's message does not show.
                if (reauth) Graph.toast(message(e))
            }
        }
    }

    private suspend fun find() {
        _step.value = SignInStep.Finding
        val (account, homes) = Graph.hf.discover()
        // One Space that answers as Launchpad: no screen.
        val sure = homes.filter { it.answers }
        _step.value = when {
            sure.size == 1 -> SignInStep.Name(account, sure.first())
            homes.size == 1 -> SignInStep.Name(account, homes.first())
            else -> SignInStep.Pick(account, homes)
        }
    }

    fun pick(home: FoundHome) {
        val s = _step.value as? SignInStep.Pick ?: return
        _step.value = SignInStep.Name(s.account, home)
    }

    /** Looks again for Spaces (after tagging one, or waking it). */
    fun findAgain() {
        cancel()
        job = Graph.scope.launch {
            try { find() } catch (e: CancellationException) { throw e } catch (e: Exception) { _step.value = SignInStep.Start(message(e)) }
        }
    }

    fun connect(name: String) {
        val s = _step.value as? SignInStep.Name ?: return
        val clean = name.trim().split(Regex("\\s+")).joinToString(" ")
        if (clean.isEmpty() || clean.length > 80) {
            _step.value = s.copy(error = "Use a name of 1 to 80 characters")
            return
        }
        _step.value = SignInStep.Connecting(s.account, s.home, clean)
        job = Graph.scope.launch {
            try {
                val answer = Graph.home.connectPhone(s.home.origin, clean)
                Graph.store.setLink(HomeLink(origin = s.home.origin, space = s.home.space, deviceId = answer.str("id") ?: "", deviceName = answer.str("name") ?: clean, account = s.account))
                Graph.store.setSecrets(Graph.store.secrets.value.copy(device = answer.str("device") ?: ""))
                _step.value = SignInStep.Start()
                Graph.onLinked()
            } catch (e: CancellationException) {
                throw e
            } catch (e: HomeException) {
                _step.value = s.copy(error = if (e.uncertain) "Home did not answer in time. If “${clean}” shows in Settings → Devices, revoke it there, then connect again." else e.message)
            } catch (e: Exception) {
                _step.value = s.copy(error = message(e))
            }
        }
    }

    /** Back on a step, or the Custom Tab was closed: nothing was saved on Home. */
    fun cancel() {
        job?.cancel()
        job = null
        pending?.cancel()
        pending = null
        if (_step.value !is SignInStep.Start) _step.value = SignInStep.Start()
    }

    private fun message(e: Exception): String = when (e) {
        is HfTransient -> "Hugging Face is unreachable. Check the connection, then try again."
        is HfRefused -> e.message ?: "Hugging Face did not sign Launchpad in."
        else -> e.message ?: "Something went wrong. Try again."
    }
}
