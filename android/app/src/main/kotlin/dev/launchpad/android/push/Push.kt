package dev.launchpad.android.push

import android.app.NotificationChannel
import android.app.NotificationChannelGroup
import android.app.NotificationManager
import android.content.Context
import com.google.firebase.FirebaseApp
import com.google.firebase.FirebaseOptions
import com.google.firebase.messaging.FirebaseMessaging
import dev.launchpad.android.BuildConfig
import dev.launchpad.android.Graph
import dev.launchpad.android.net.HomeException
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject

/**
 * Push: Home wakes this phone through Firebase Cloud Messaging with a data message that holds no text
 * (`{kind: "wake", ws, n}`); [Notifier] then reads the notifications from Home. Firebase is the owner's own project,
 * baked in at build time (`LAUNCHPAD_FIREBASE_*`): a build without it never starts Firebase and has no push, and
 * everything else works.
 */
object Push {
    const val CH_NEEDS_YOU = "needs_you"
    const val CH_CHATS = "chats"
    const val CH_DONE = "done"
    const val CH_CONNECTION = "connection"
    const val CH_UPDATES = "updates"

    val configured: Boolean
        get() = BuildConfig.FIREBASE_PROJECT_ID.isNotBlank() && BuildConfig.FIREBASE_APP_ID.isNotBlank() && BuildConfig.FIREBASE_API_KEY.isNotBlank() && BuildConfig.FIREBASE_SENDER_ID.isNotBlank()

    /** At process start: the notification channels, and Firebase when this build has it. */
    fun init(context: Context) {
        channels(context)
        if (!configured || FirebaseApp.getApps(context).isNotEmpty()) return
        val options = FirebaseOptions.Builder()
            .setProjectId(BuildConfig.FIREBASE_PROJECT_ID)
            .setApplicationId(BuildConfig.FIREBASE_APP_ID)
            .setApiKey(BuildConfig.FIREBASE_API_KEY)
            .setGcmSenderId(BuildConfig.FIREBASE_SENDER_ID)
            .build()
        runCatching {
            FirebaseApp.initializeApp(context, options)
            // Off in the manifest so nothing starts before this build's own project is set.
            FirebaseMessaging.getInstance().isAutoInitEnabled = true
        }
    }

    private fun channels(context: Context) {
        val nm = context.getSystemService(NotificationManager::class.java)
        nm.createNotificationChannelGroup(NotificationChannelGroup("launchpad", "Launchpad"))
        fun ch(id: String, name: String, importance: Int, description: String) = NotificationChannel(id, name, importance).apply {
            this.description = description
            group = "launchpad"
            // Questions and replies may hold work text: hidden on the lock screen until unlock.
            lockscreenVisibility = android.app.Notification.VISIBILITY_PRIVATE
        }
        nm.createNotificationChannels(listOf(
            ch(CH_NEEDS_YOU, "Needs you", NotificationManager.IMPORTANCE_HIGH, "An agent asks a question, failed or was stopped by quota; a report asks you"),
            ch(CH_CHATS, "Bots and chats", NotificationManager.IMPORTANCE_DEFAULT, "A bot answered, asks you in a channel, or proposes a card"),
            ch(CH_DONE, "Done", NotificationManager.IMPORTANCE_LOW, "An agent finished, a routine reported, quota resumed agents"),
            ch(CH_CONNECTION, "Connection", NotificationManager.IMPORTANCE_LOW, "Sign in again, this phone was removed, Home or the app is too old"),
            ch(CH_UPDATES, "App updates", NotificationManager.IMPORTANCE_LOW, "A newer Launchpad app is on GitHub"),
        ))
    }

    /** Registers this phone's FCM token with Home (after sign-in, at start, and when FCM replaces it). */
    fun register() {
        if (!configured || !Graph.linked.value) return
        runCatching {
            FirebaseMessaging.getInstance().token.addOnSuccessListener { token -> sendToken(token) }
        }
    }

    fun sendToken(token: String) {
        if (!Graph.linked.value || token.isBlank()) return
        Graph.scope.launch {
            try {
                Graph.home.write("PUT", "/api/devices/self/push", buildJsonObject { put("token", JsonPrimitive(token)) })
            } catch (_: HomeException) {
                // Home is too old (no `device_push`) or unreachable: the next start registers again.
            }
        }
    }

    /** Notifications off on this phone: Home forgets the token (sign-out deletes it with the device anyway). */
    fun unregister() {
        if (!configured) return
        Graph.scope.launch {
            runCatching { Graph.home.write("DELETE", "/api/devices/self/push") }
        }
    }
}
