package dev.launchpad.android.push

import android.Manifest
import android.app.Notification
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import androidx.core.app.NotificationCompat
import androidx.core.app.RemoteInput
import androidx.core.content.ContextCompat
import dev.launchpad.android.Graph
import dev.launchpad.android.MainActivity
import dev.launchpad.android.R
import dev.launchpad.android.data.LpJson
import dev.launchpad.android.data.str
import dev.launchpad.android.gen.PhoneNotification
import dev.launchpad.android.gen.PhoneNotifications
import dev.launchpad.android.gen.Question
import dev.launchpad.android.gen.SessionEventPage
import dev.launchpad.android.net.Failure
import dev.launchpad.android.net.HomeException
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.builtins.MapSerializer
import kotlinx.serialization.builtins.serializer
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject

/** What a notification's tap opens: `session:<id>`, `thread:<bot>:<thread>`, `channel:<id>`, `card:<item id>`. */
const val EXTRA_OPEN = "dev.launchpad.android.OPEN"

/**
 * Builds the phone's notifications from Home's list (`GET /api/phone/notifications`), after a wake-up or when the app
 * comes to the front. Each item is one notification (tag: its ID), replaced when its time changes and removed when
 * it leaves the list (answered on the laptop, seen, dismissed). Grouped per workspace. Buttons: a question's options
 * and Reply, Reply for bot answers, channel asks and report questions, Seen for finished and failed agents; never
 * Merge or Publish.
 */
object Notifier {
    private const val PREFS = "notifications"
    private const val KEY_SHOWN = "shown"
    private const val UNREACHABLE = "home-unreachable"
    private val lock = Mutex()

    fun refreshBlocking(context: Context) {
        runBlocking { withTimeoutOrNull(9_000) { refresh(context) } ?: unreachable(context) }
    }

    /** Reads the list and brings the shade in line with it. */
    suspend fun refresh(context: Context): Boolean = lock.withLock {
        if (!Graph.linked.value) return false
        val list = try {
            Graph.home.get("/api/phone/notifications", PhoneNotifications.serializer())
        } catch (e: HomeException) {
            when (e.failure) {
                Failure.Revoked -> connection(context, "This phone was removed from Home", "Open Launchpad to connect it again.")
                Failure.AuthExpired -> connection(context, "Sign in again", "Hugging Face no longer accepts this phone's sign-in.")
                else -> unreachable(context)
            }
            return false
        }
        val nm = context.getSystemService(NotificationManager::class.java)
        nm.cancel(UNREACHABLE, 0)
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val shown: Map<String, Double> = prefs.getString(KEY_SHOWN, null)?.let { runCatching { LpJson.decodeFromString(MapSerializer(String.serializer(), Double.serializer()), it) }.getOrNull() } ?: emptyMap()
        val now = list.items.associateBy { it.id }
        // Gone from Home's list: answered or seen elsewhere.
        for (id in shown.keys - now.keys) nm.cancel(id, 0)
        if (allowed(context)) {
            for (item in list.items) {
                if (shown[item.id]?.let { it >= item.at } == true) continue
                nm.notify(item.id, 0, build(context, item))
            }
            summaries(context, list.items)
        }
        prefs.edit().putString(KEY_SHOWN, LpJson.encodeToString(MapSerializer(String.serializer(), Double.serializer()), list.items.associate { it.id to it.at })).apply()
        true
    }

    /** One notification's own work is done (answered, replied, seen from the shade): it goes at once. */
    fun done(context: Context, id: String) {
        context.getSystemService(NotificationManager::class.java).cancel(id, 0)
    }

    /** Signed out or removed: every notification and the list of shown ones go. */
    fun wipe(context: Context) {
        context.getSystemService(NotificationManager::class.java).cancelAll()
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().clear().apply()
    }

    fun test(context: Context) {
        if (!allowed(context)) return
        val n = NotificationCompat.Builder(context, Push.CH_CONNECTION)
            .setSmallIcon(R.drawable.ic_launcher_foreground)
            .setContentTitle("Launchpad test")
            .setContentText("Home's wake-ups reach this phone.")
            .setContentIntent(open(context, "test", null))
            .setAutoCancel(true)
            .build()
        context.getSystemService(NotificationManager::class.java).notify("test", 0, n)
    }

    private fun allowed(context: Context): Boolean =
        android.os.Build.VERSION.SDK_INT < 33 || ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED

    /** Woken, but Home did not answer: a notification without content, so nothing waits unseen. */
    private fun unreachable(context: Context) {
        if (!allowed(context)) return
        val n = NotificationCompat.Builder(context, Push.CH_NEEDS_YOU)
            .setSmallIcon(R.drawable.ic_launcher_foreground)
            .setContentTitle("Launchpad: something needs you")
            .setContentText("Home did not answer. Open Launchpad to see what changed.")
            .setContentIntent(open(context, UNREACHABLE, null))
            .setAutoCancel(true)
            .build()
        context.getSystemService(NotificationManager::class.java).notify(UNREACHABLE, 0, n)
    }

    private fun connection(context: Context, title: String, text: String) {
        if (!allowed(context)) return
        val n = NotificationCompat.Builder(context, Push.CH_CONNECTION)
            .setSmallIcon(R.drawable.ic_launcher_foreground)
            .setContentTitle(title)
            .setContentText(text)
            .setContentIntent(open(context, "connection", null))
            .setAutoCancel(true)
            .build()
        context.getSystemService(NotificationManager::class.java).notify("connection", 0, n)
    }

    private fun target(item: PhoneNotification): String? = when {
        item.bot_id != null && item.thread_id != null -> "thread:${item.bot_id}:${item.thread_id}"
        item.channel_id != null -> "channel:${item.channel_id}"
        item.session_id != null -> "session:${item.session_id}"
        item.proposal_id != null -> "card:proposal:${item.proposal_id}"
        else -> null
    }

    fun open(context: Context, id: String, target: String?): PendingIntent {
        val intent = Intent(context, MainActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP)
            .putExtra(EXTRA_OPEN, target ?: "")
        return PendingIntent.getActivity(context, ("open:$id").hashCode(), intent, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
    }

    private fun group(item: PhoneNotification) = "ws:${item.workspace_id ?: ""}"

    private suspend fun build(context: Context, item: PhoneNotification): Notification {
        val b = NotificationCompat.Builder(context, item.channel.ifBlank { Push.CH_CHATS })
            .setSmallIcon(R.drawable.ic_launcher_foreground)
            .setContentTitle(item.title.ifBlank { label(item.kind) })
            .setContentText(item.text.ifBlank { label(item.kind) })
            .setStyle(NotificationCompat.BigTextStyle().bigText(item.text.ifBlank { label(item.kind) }))
            .setSubText(label(item.kind))
            .setWhen(item.at.toLong())
            .setShowWhen(true)
            .setGroup(group(item))
            .setContentIntent(open(context, item.id, target(item)))
            .setAutoCancel(true)
            .setOnlyAlertOnce(true)
            .setCategory(if (item.channel == Push.CH_NEEDS_YOU) NotificationCompat.CATEGORY_REMINDER else NotificationCompat.CATEGORY_MESSAGE)
            .setPublicVersion(
                NotificationCompat.Builder(context, item.channel.ifBlank { Push.CH_CHATS })
                    .setSmallIcon(R.drawable.ic_launcher_foreground)
                    .setContentTitle("Launchpad")
                    .setContentText(label(item.kind))
                    .build()
            )
        when (item.kind) {
            "question" -> {
                item.session_id?.let { sid ->
                    val ask = openQuestion(sid)
                    val q = ask?.second?.singleOrNull()
                    if (ask != null && q != null && !q.multi_select) {
                        q.options.take(3).forEach { o -> b.addAction(0, o.label.take(30), ActionReceiver.answer(context, item, ask.first, o.label)) }
                    }
                    if (ask != null) b.addAction(reply(context, item, ActionReceiver.ANSWER_TEXT, "Answer", ask.first))
                }
            }
            "bot_answer", "channel_ask", "report_question" -> b.addAction(reply(context, item, ActionReceiver.REPLY, "Reply", null))
            "failed", "finished", "quota_stop", "quota_resumed" -> b.addAction(0, "Seen", ActionReceiver.seen(context, item))
        }
        return b.build()
    }

    private fun reply(context: Context, item: PhoneNotification, kind: String, label: String, requestId: String?): NotificationCompat.Action {
        val input = RemoteInput.Builder(ActionReceiver.KEY_TEXT).setLabel(label).build()
        return NotificationCompat.Action.Builder(0, label, ActionReceiver.withText(context, item, kind, requestId))
            .addRemoteInput(input)
            .setAllowGeneratedReplies(false)
            .build()
    }

    /** The open question of a session: its request ID and questions, from the end of its trace. */
    private suspend fun openQuestion(sessionId: String): Pair<String, List<Question>>? = runCatching {
        val page = Graph.home.get("/api/sessions/$sessionId/events/page?tail=80", SessionEventPage.serializer())
        var open: Pair<String, List<Question>>? = null
        for (e in page.events) {
            when (e.body.str("kind")) {
                "ask_user" -> {
                    val qs = LpJson.decodeFromJsonElement(ListSerializer(Question.serializer()), (e.body as JsonObject)["questions"] ?: JsonArray(emptyList()))
                    open = (e.body.str("request_id") ?: "") to qs
                }
                "ask_answered" -> if (e.body.str("request_id") == open?.first) open = null
            }
        }
        open?.takeIf { it.first.isNotBlank() }
    }.getOrNull()

    /** One summary per workspace with more than one notification, so Android groups them. */
    private fun summaries(context: Context, items: List<PhoneNotification>) {
        val nm = context.getSystemService(NotificationManager::class.java)
        for ((g, members) in items.groupBy { group(it) }) {
            if (members.size < 2) {
                nm.cancel("summary:$g", 0)
                continue
            }
            val s = NotificationCompat.Builder(context, Push.CH_CHATS)
                .setSmallIcon(R.drawable.ic_launcher_foreground)
                .setContentTitle("Launchpad")
                .setContentText("${members.size} waiting")
                .setGroup(g)
                .setGroupSummary(true)
                .setSilent(true)
                .setContentIntent(open(context, "summary:$g", null))
                .setAutoCancel(true)
                .build()
            nm.notify("summary:$g", 0, s)
        }
    }

    fun label(kind: String): String = when (kind) {
        "question" -> "An agent asks"
        "failed" -> "Failed"
        "routine" -> "A routine failed"
        "report_question" -> "A report asks you"
        "channel_ask" -> "A bot asks you"
        "bot_answer" -> "A bot answered"
        "card" -> "A card waits"
        "finished" -> "Done"
        "report" -> "Report"
        "quota_stop" -> "Stopped automatically"
        "quota_resumed" -> "Resumed"
        else -> "Launchpad"
    }
}
