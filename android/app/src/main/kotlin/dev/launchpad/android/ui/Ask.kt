package dev.launchpad.android.ui

import android.content.Context
import dev.launchpad.android.Graph
import dev.launchpad.android.data.SendState
import dev.launchpad.android.data.Sends
import dev.launchpad.android.gen.FileAttachment
import kotlinx.coroutines.launch

/**
 * The Ask box's send (A-6): to a bot, a new thread of it, whose chat then opens; or a new agent with Home's defaults
 * (the workspace's latest agent's profile and folder), whose screen then opens. Without an agent to copy defaults
 * from, the composer opens in the browser with the text. An uncertain send is never repeated: the owner checks.
 */
fun ask(context: Context, target: AskTarget, text: String, files: List<FileAttachment>) {
    val workspace = Graph.repo.workspaceId.value
    Graph.scope.launch {
        when (target) {
            is AskTarget.ToBot -> {
                Graph.toast("Sending to ${target.label}…")
                val o = Sends.deliver(Sends.botMessage(target.bot.id, null, text, files))
                val thread = Sends.receipt(o)?.thread
                when {
                    o.state == SendState.Sent && !thread.isNullOrEmpty() -> {
                        Graph.repo.invalidate("threads", "sessions")
                        Graph.nav.open(Route.Conversation(target.bot.id, thread, target.bot.settings.name))
                    }
                    o.state == SendState.Sent -> Graph.nav.open(Route.Conversation(target.bot.id, "main", target.bot.settings.name))
                    o.state == SendState.Uncertain -> Graph.toast("No answer from Home: check ${target.label}'s chats before sending again")
                    else -> Graph.toast("Not sent: ${o.error ?: "no connection"}")
                }
            }
            AskTarget.NewAgent -> {
                val spec = Sends.launch(workspace, text, files)
                if (spec == null) {
                    openInBrowser(context, "/?share_text=" + java.net.URLEncoder.encode(text, "UTF-8") + "&ws=" + workspace)
                    return@launch
                }
                Graph.toast("Starting an agent…")
                val o = Sends.deliver(spec)
                val session = Sends.launched(o)
                when {
                    session != null -> { Graph.repo.invalidate("sessions"); Graph.nav.open(Route.Agent(session.id, text.lineSequence().first().take(80))) }
                    o.state == SendState.Uncertain -> Graph.toast("No answer from Home: check Agents before starting it again")
                    else -> Graph.toast("Not started: ${o.error ?: "no connection"}")
                }
            }
        }
    }
}
