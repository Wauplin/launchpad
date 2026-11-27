package dev.launchpad.android.data

import dev.launchpad.android.Graph
import dev.launchpad.android.gen.BotSend
import dev.launchpad.android.gen.FileAttachment
import dev.launchpad.android.gen.Session
import dev.launchpad.android.net.Failure
import dev.launchpad.android.net.HomeException
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import java.net.URLEncoder
import java.util.UUID

// Everything the owner writes from the phone: replies to bots, agents and channels, answers, card replies and
// launches. Rules (design.md "reply", live.md "Seen, presence and sends"):
// - each send has its own `Idempotency-Key` (a UUID) and keeps its exact body, so "Send again" and "Check" repeat the
//   same request: Home's retry protection (crates/server/src/operations.rs) answers a bot message, a session reply or
//   a launch it already did with the stored result, never a second message;
// - a send that never left the phone is Not sent; one whose answer never came is Uncertain and is never repeated
//   automatically (only by the owner's Check);
// - Home updating (503 `retryable`) is not uncertain: `HomeClient.write` waits and sends once Home is back.

enum class SendState { Sending, Sent, NotSent, Uncertain }

/** One send of the owner: what it was (text), how to repeat it exactly (method, path, body, key), and where it is. */
data class Outgoing(
    val key: String,
    val text: String,
    val method: String,
    val path: String,
    val body: JsonElement,
    val state: SendState = SendState.Sending,
    val error: String? = null,
    /** Home's answer once sent (a BotSend, a Session, `{ok}`). */
    val answer: JsonElement? = null,
    /** Home protects this route against repeats (operations.rs): Check sends it again with the same key. */
    val protected: Boolean = false,
    val at: Long = System.currentTimeMillis(),
)

private fun enc(s: String) = URLEncoder.encode(s, "UTF-8").replace("+", "%20")

object Sends {
    fun newKey(): String = UUID.randomUUID().toString()

    /**
     * Runs a send once and answers it with its new state. Repeating an [Outgoing] (Send again, Check) calls this
     * again with the same key and body.
     */
    suspend fun deliver(o: Outgoing): Outgoing = try {
        val answer = Graph.home.write(o.method, o.path, o.body, if (o.protected) o.key else null)
        o.copy(state = SendState.Sent, error = null, answer = answer)
    } catch (e: HomeException) {
        when {
            !e.sent -> o.copy(state = SendState.NotSent, error = e.message)
            e.uncertain -> o.copy(state = SendState.Uncertain, error = "No answer from Home: it may have arrived")
            // A protected send that Home saw but could not finish: Home says so and will not repeat it.
            e.status == 409 && o.protected -> o.copy(state = SendState.Uncertain, error = e.message)
            e.failure == Failure.Network -> o.copy(state = SendState.NotSent, error = e.message)
            else -> o.copy(state = SendState.NotSent, error = e.message)
        }
    }

    // ---------------------------------------------------------------- the sends, as data (deliver them with [deliver])

    private fun files(attachments: List<FileAttachment>): JsonArray = buildJsonArray {
        attachments.forEach { a -> add(buildJsonObject { put("name", a.name); put("data_url", a.data_url) }) }
    }

    /**
     * The owner's message to a bot (`bot.message` with `accept`: Home stores it and answers with its receipt, a
     * [BotSend], whose `thread` is where it went). `thread` null: a new thread (the Ask box).
     */
    fun botMessage(botId: String, thread: String?, text: String, attachments: List<FileAttachment> = emptyList(), key: String = newKey()) = Outgoing(
        key, text, "POST", "/api/bots/${enc(botId)}/message",
        buildJsonObject {
            put("text", text)
            put("attachments", files(attachments))
            if (thread.isNullOrEmpty()) put("new_thread", true) else put("thread", thread)
            put("accept", true)
        },
        protected = true,
    )

    /** A reply to an agent (`session.message`). */
    fun sessionInput(sessionId: String, text: String, attachments: List<FileAttachment> = emptyList(), key: String = newKey()) = Outgoing(
        key, text, "POST", "/api/sessions/${enc(sessionId)}/input",
        buildJsonObject { put("text", text); put("files", files(attachments)) },
        protected = true,
    )

    /** An answer to an open question (`session.answer`): one answer per question. Protected by Home. */
    fun answer(sessionId: String, requestId: String, answers: List<String>, key: String = newKey()) = Outgoing(
        key, answers.joinToString(" · "), "POST", "/api/sessions/${enc(sessionId)}/answer",
        buildJsonObject { put("request_id", requestId); put("answers", buildJsonArray { answers.forEach { add(JsonPrimitive(it)) } }) },
        protected = true,
    )

    /** The owner's post in a channel or DM (`channel.post`); `replyTo` answers a bot's ask. Protected by Home. */
    fun channelPost(channelId: String, text: String, replyTo: Long? = null, key: String = newKey()) = Outgoing(
        key, text, "POST", "/api/channels/${enc(channelId)}/messages",
        buildJsonObject { put("text", text); if (replyTo != null) put("reply_to", replyTo) },
        protected = true,
    )

    /** A reply to a report card that asks (`proposal.reply`). Protected by Home. */
    fun proposalReply(proposalId: String, text: String, key: String = newKey()) = Outgoing(
        key, text, "POST", "/api/proposals/${enc(proposalId)}/reply", buildJsonObject { put("text", text) },
        protected = true,
    )

    /**
     * A new agent with Home's defaults (`session.launch`): the profile, model, effort and permission mode of the
     * workspace's latest agent, in its folder, in a new task worktree. Null: no agent yet to copy them from (the
     * composer in the browser chooses).
     */
    fun launch(workspace: String, text: String, attachments: List<FileAttachment> = emptyList(), key: String = newKey()): Outgoing? {
        val repo = Graph.repo
        val last = agents(workspace, repo.sessions.value, repo.projects.value).maxByOrNull { it.started_at } ?: return null
        val spec = last.launch_spec
        val project = last.project_id ?: spec.target.str("project_id")
        val body = buildJsonObject {
            put("text", text)
            put("attachments", files(attachments))
            spec.harness?.let { put("harness", it) }
            spec.profile?.let { put("profile", it) }
            spec.model?.let { put("model", it) }
            spec.effort?.let { put("effort", it) }
            if (spec.permission_mode.isNotEmpty()) put("permission_mode", spec.permission_mode)
            put("allow_paid_usage", false)
            put("workspace_id", workspace)
            if (project != null) {
                put("target", buildJsonObject { put("kind", "project"); put("project_id", project) })
                put("worktree", buildJsonObject { put("base", JsonNull); put("when_done", "keep") })
            } else put("target", buildJsonObject { put("kind", "temp") })
        }
        return Outgoing(key, text, "POST", "/api/sessions", body, protected = true)
    }

    // ---------------------------------------------------------------- one-shot calls for screens and notifications

    /** Uploads one file once (`POST /api/uploads`); the attachment carries Home's short reference. */
    suspend fun upload(name: String, contentType: String, bytes: ByteArray): FileAttachment {
        val r = Graph.home.raw("POST", "/api/uploads", bytes, contentType)
        val json = runCatching { LpJson.parseToJsonElement(String(r.bytes)) }.getOrNull()
        val url = json.str("data_url")
        if (r.status >= 400 || url == null) throw HomeException(Failure.Other, json.str("error") ?: "Upload failed (${r.status})", r.status)
        return FileAttachment(name = name, data_url = url)
    }

    /** The session Home answered a launch with. */
    fun launched(o: Outgoing): Session? = o.answer?.let { runCatching { LpJson.decodeFromJsonElement(Session.serializer(), it) }.getOrNull() }

    /** The receipt Home answered a bot message with. */
    fun receipt(o: Outgoing): BotSend? = o.answer?.let { runCatching { LpJson.decodeFromJsonElement(BotSend.serializer(), it) }.getOrNull() }

    /** Marks a session seen (`session.seen`), quietly. */
    suspend fun seen(sessionId: String) {
        runCatching { Graph.home.write("POST", "/api/sessions/${enc(sessionId)}/seen") }
    }

    /** Marks a channel read up to a message (`channel.read`), quietly. */
    suspend fun channelRead(channelId: String, through: Long?) {
        runCatching { Graph.home.write("POST", "/api/channels/${enc(channelId)}/read", buildJsonObject { if (through != null) put("through", through) }) }
    }

    /** Runs an action of the action list for a button; a failure becomes a toast. True when it ran. */
    suspend fun action(name: String, params: Map<String, String> = emptyMap(), input: JsonObject? = null, done: String? = null): Boolean = try {
        Graph.home.runAction(name, params, input)
        done?.let { Graph.toast(it) }
        true
    } catch (e: HomeException) {
        Graph.toast(if (e.uncertain) "No answer from Home: check before trying again" else e.message ?: "Home did not answer")
        false
    }
}
