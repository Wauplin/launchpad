package dev.launchpad.android.push

import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage
import dev.launchpad.android.Graph

/**
 * Wake-ups from Home. The message holds `kind` ("wake" or "test"), `ws` and `n` only: never a title or text. The
 * notifications themselves come from Home ([Notifier.refresh]), over this phone's own connection.
 */
class MessagingService : FirebaseMessagingService() {
    override fun onNewToken(token: String) {
        Graph.init(application)
        Push.sendToken(token)
    }

    override fun onMessageReceived(message: RemoteMessage) {
        Graph.init(application)
        when (message.data["kind"]) {
            "test" -> Notifier.test(this)
            // A high-priority message gives about 10 s of network: read Home now, on this thread.
            else -> Notifier.refreshBlocking(this)
        }
    }
}
