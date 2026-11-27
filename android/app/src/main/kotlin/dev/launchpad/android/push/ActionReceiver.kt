package dev.launchpad.android.push

import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import androidx.core.app.NotificationCompat
import androidx.core.app.RemoteInput
import dev.launchpad.android.Graph
import dev.launchpad.android.R
import dev.launchpad.android.gen.PhoneNotification
import dev.launchpad.android.net.HomeException
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject

/**
 * The buttons of a notification: an answer, an inline reply or Seen, each one action of the action list, sent once
 * with an idempotency key made of the notification and its time (a second tap is done once on Home). A send that
 * fails or has no answer is shown on the notification and never repeated by itself.
 */
class ActionReceiver : BroadcastReceiver() {
    companion object {
        const val KEY_TEXT = "text"
        const val ANSWER = "answer"
        const val ANSWER_TEXT = "answer_text"
        const val REPLY = "reply"
        const val SEEN = "seen"

        private fun intent(context: Context, item: PhoneNotification, kind: String, extra: Intent.() -> Unit): Intent =
            Intent(context, ActionReceiver::class.java).setAction("dev.launchpad.android.$kind:${item.id}:${item.at.toLong()}")
                .putExtra("kind", kind).putExtra("id", item.id).putExtra("at", item.at.toLong()).putExtra("notification_kind", item.kind)
                .putExtra("session", item.session_id).putExtra("bot", item.bot_id).putExtra("thread", item.thread_id)
                .putExtra("channel", item.channel_id).putExtra("message", item.message?.toLong() ?: -1L).putExtra("proposal", item.proposal_id)
                .apply(extra)

        fun answer(context: Context, item: PhoneNotification, requestId: String, label: String): PendingIntent {
            val i = intent(context, item, ANSWER) { putExtra("request", requestId); putExtra("label", label) }
            return PendingIntent.getBroadcast(context, (i.action + label).hashCode(), i, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        }

        fun seen(context: Context, item: PhoneNotification): PendingIntent {
            val i = intent(context, item, SEEN) {}
            return PendingIntent.getBroadcast(context, i.action.hashCode(), i, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        }

        /** Inline replies need a mutable intent: Android adds the typed text to it. */
        fun withText(context: Context, item: PhoneNotification, kind: String, requestId: String?): PendingIntent {
            val i = intent(context, item, kind) { putExtra("request", requestId) }
            return PendingIntent.getBroadcast(context, i.action.hashCode(), i, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_MUTABLE)
        }
    }

    override fun onReceive(context: Context, intent: Intent) {
        Graph.init(context.applicationContext as android.app.Application)
        val pending = goAsync()
        val id = intent.getStringExtra("id") ?: return pending.finish()
        val kind = intent.getStringExtra("kind") ?: return pending.finish()
        val text = RemoteInput.getResultsFromIntent(intent)?.getCharSequence(KEY_TEXT)?.toString()?.trim()
        // Home takes a lowercase UUID: the same notification and button give the same key, so a second tap is done once.
        val key = java.util.UUID.nameUUIDFromBytes(("phone:$kind:$id:${intent.getLongExtra("at", 0)}" + (intent.getStringExtra("label")?.let { ":$it" } ?: "")).toByteArray()).toString()
        Graph.scope.launch {
            try {
                run(intent, kind, text, key)
                Notifier.done(context, id)
                Graph.scope.launch { runCatching { Notifier.refresh(context) } }
            } catch (e: HomeException) {
                failed(context, intent, if (e.uncertain) "No answer from Home: open Launchpad to check before sending again." else "Not sent: ${e.message}")
            } catch (e: IllegalArgumentException) {
                failed(context, intent, e.message ?: "Not sent")
            } finally {
                pending.finish()
            }
        }
    }

    private suspend fun run(intent: Intent, kind: String, text: String?, key: String) {
        val session = intent.getStringExtra("session")
        when (kind) {
            ANSWER, ANSWER_TEXT -> {
                val answer = if (kind == ANSWER) intent.getStringExtra("label") else text
                require(!answer.isNullOrBlank() && session != null) { "Nothing to send" }
                Graph.home.runAction("session.answer", mapOf("id" to session), buildJsonObject {
                    put("request_id", JsonPrimitive(intent.getStringExtra("request") ?: ""))
                    put("answers", JsonArray(listOf(JsonPrimitive(answer))))
                }, key)
            }
            REPLY -> {
                require(!text.isNullOrBlank()) { "Nothing to send" }
                val bot = intent.getStringExtra("bot")
                val channel = intent.getStringExtra("channel")
                val proposal = intent.getStringExtra("proposal")
                when {
                    bot != null -> Graph.home.runAction("bot.message", mapOf("id" to bot), buildJsonObject {
                        put("text", JsonPrimitive(text))
                        intent.getStringExtra("thread")?.let { put("thread", JsonPrimitive(it)) }
                    }, key)
                    channel != null -> Graph.home.runAction("channel.post", mapOf("id" to channel), buildJsonObject {
                        put("text", JsonPrimitive(text))
                        intent.getLongExtra("message", -1L).takeIf { it >= 0 }?.let { put("reply_to", JsonPrimitive(it)) }
                    }, key)
                    proposal != null -> Graph.home.runAction("proposal.reply", mapOf("id" to proposal), buildJsonObject { put("text", JsonPrimitive(text)) }, key)
                    else -> throw IllegalArgumentException("Open Launchpad to reply")
                }
            }
            SEEN -> {
                val id = intent.getStringExtra("id") ?: ""
                if (id.startsWith("quota:")) Graph.home.write("POST", "/api/phone/notifications/dismiss", buildJsonObject { put("id", JsonPrimitive(id)) }, key)
                else if (session != null) Graph.home.runAction("session.seen", mapOf("id" to session), null, key)
            }
        }
    }

    /** The send failed: the notification says so (with its own buttons gone), never retried. */
    private fun failed(context: Context, intent: Intent, why: String) {
        val id = intent.getStringExtra("id") ?: return
        val n = NotificationCompat.Builder(context, Push.CH_NEEDS_YOU)
            .setSmallIcon(R.drawable.ic_launcher_foreground)
            .setContentTitle(Notifier.label(intent.getStringExtra("notification_kind") ?: ""))
            .setContentText(why)
            .setStyle(NotificationCompat.BigTextStyle().bigText(why))
            .setOnlyAlertOnce(true)
            .setAutoCancel(true)
            .setContentIntent(Notifier.open(context, "failed:$id", intent.getStringExtra("session")?.let { "session:$it" }))
            .build()
        context.getSystemService(NotificationManager::class.java).notify(id, 0, n)
    }
}
