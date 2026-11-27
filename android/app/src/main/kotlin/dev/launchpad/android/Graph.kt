package dev.launchpad.android

import android.app.Application
import android.content.Context
import android.content.Intent
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.ProcessLifecycleOwner
import dev.launchpad.android.data.Cache
import dev.launchpad.android.data.Repository
import dev.launchpad.android.data.Store
import dev.launchpad.android.net.EventStream
import dev.launchpad.android.net.Failure
import dev.launchpad.android.net.HfAuth
import dev.launchpad.android.net.HomeClient
import dev.launchpad.android.net.HomeException
import dev.launchpad.android.ui.Navigator
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import okhttp3.OkHttpClient
import java.util.concurrent.TimeUnit

class LaunchpadApp : Application() {
    override fun onCreate() {
        super.onCreate()
        Graph.init(this)
    }
}

/**
 * The app's objects, created once per process: `Graph.home` calls Home, `Graph.stream` is the live bus, `Graph.repo`
 * holds the lists, `Graph.nav` the screens. Screens read these directly (one user, one Home, one process).
 */
object Graph {
    lateinit var context: Context private set
    lateinit var store: Store private set
    lateinit var cache: Cache private set
    lateinit var hf: HfAuth private set
    lateinit var home: HomeClient private set
    lateinit var stream: EventStream private set
    lateinit var repo: Repository private set
    lateinit var signIn: SignInFlow private set
    val nav = Navigator()
    val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

    private val _linked = MutableStateFlow(false)
    /** This phone is a device of a Home: false shows Connect. */
    val linked: StateFlow<Boolean> = _linked
    /** Why Connect shows again ("This phone was removed from Home"), until the next sign-in. */
    val notice = MutableStateFlow<String?>(null)
    /** One-line messages for the current screen's snackbar. */
    val toasts = kotlinx.coroutines.flow.MutableSharedFlow<String>(extraBufferCapacity = 8)
    private var visible = false

    fun init(app: Application) {
        if (::context.isInitialized) return
        context = app
        store = Store(app)
        cache = Cache(app)
        // No cookies (OkHttp keeps none by default), no proxy secrets.
        val http = OkHttpClient.Builder().connectTimeout(10, TimeUnit.SECONDS).readTimeout(30, TimeUnit.SECONDS).writeTimeout(30, TimeUnit.SECONDS).build()
        hf = HfAuth(http, store)
        home = HomeClient(http, store, hf)
        // Not in the stream's own job: forgetting stops it.
        stream = EventStream(app, home, scope) { scope.launch { removed() } }
        repo = Repository(home, stream, cache, store, scope)
        signIn = SignInFlow()
        _linked.value = store.linked
        // Notification channels, and Firebase when this build has it (else no push, everything else works).
        dev.launchpad.android.push.Push.init(app)
        dev.launchpad.android.update.UpdateCheck.schedule(app)
        scope.launch {
            home.failures.collect { failure ->
                when (failure) {
                    Failure.Revoked -> removed()
                    // The stream finds out by itself; a screen's own call may find out first.
                    Failure.AuthExpired -> stream.markAuthExpired()
                    else -> {}
                }
            }
        }
        ProcessLifecycleOwner.get().lifecycle.addObserver(object : DefaultLifecycleObserver {
            override fun onStart(owner: LifecycleOwner) {
                visible = true
                if (_linked.value) {
                    stream.setVisible(true)
                    // The shade follows Home's list also without a wake-up (answered on the laptop meanwhile).
                    scope.launch { runCatching { dev.launchpad.android.push.Notifier.refresh(context) } }
                }
            }
            override fun onStop(owner: LifecycleOwner) {
                visible = false
                stream.setVisible(false)
            }
        })
        if (_linked.value) {
            repo.start()
            dev.launchpad.android.push.Push.register()
        }
    }

    /** Home created this phone's device (the end of Connect). */
    fun onLinked() {
        notice.value = null
        _linked.value = true
        nav.reset()
        repo.start()
        if (visible) stream.setVisible(true)
        dev.launchpad.android.push.Push.register()
    }

    /** Home refused this device (Revoke in Settings → Devices): secrets and cache are deleted, back to Connect. */
    suspend fun removed() {
        if (!_linked.value) return
        forget()
        notice.value = "This phone was removed from Home. Sign in to connect it again."
    }

    /** Sign out: Home deletes this device (and its push token), then the phone forgets everything. */
    suspend fun signOut() {
        try {
            home.write("DELETE", "/api/devices/self")
        } catch (e: HomeException) {
            // Offline: the device stays listed in Settings → Devices until it is revoked there.
            if (e.failure != Failure.Revoked) toasts.tryEmit("Home did not answer: revoke this phone in Settings → Devices")
        }
        forget()
    }

    private suspend fun forget() {
        _linked.value = false
        stream.reset()
        repo.wipe()
        // The folio reader's offline copies and reading positions.
        dev.launchpad.android.data.FolioCache.wipe()
        // Notifications of a Home this phone no longer reads.
        dev.launchpad.android.push.Notifier.wipe(context)
        store.wipe()
        nav.reset()
    }

    fun toast(message: String) { toasts.tryEmit(message) }

    /** Brings the app back in front of the Custom Tab (its task is in the foreground, so Android allows it). */
    fun bringToFront() {
        runCatching {
            context.startActivity(Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP))
        }
    }
}
