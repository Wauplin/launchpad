// Chapter 6: Ask Launchpad, a bot like the others: proposals and the owner's click; then bots, agents that stay.

import type { Chapter } from "../tour/types";
import { api } from "@/lib/api";
import { useWorkspace } from "@/stores/workspace";
import { world } from "@/demo/world";
import { WS } from "@/demo/world/ids";
import { TODAY_PROPOSAL } from "@/demo/scenarios/askToday";
import { meta } from "./meta";
import { ASK_BOT, BOT } from "@/demo/world/bots";
import { RELEASE_TRAIN } from "@/demo/world/channels";
import { useAskPanel } from "@/stores/ask";

const THREAD = "demo-s-ask-acme";
const READY = "demo-prop-ready-done";
const proposal = () => world.proposals.find((p) => p.id === TODAY_PROPOSAL);
const ASK_ROUTE = `/bots/${ASK_BOT}`;
const BOT_ROUTE = `/bots/${BOT}`;
const CHANNEL_ROUTE = `/channels/${RELEASE_TRAIN}`;
const DAILY = "demo-routine-daily";
let dailyRuns = 0;
const botSessions = () => world.sessions.filter((s) => s.launch_spec.bot?.id === BOT);

export const ask: Chapter = {
  ...meta("ask"),
  learned: ["Ask from the top-bar field on any page: the bot's chat opens as a panel, with the page as context", "Ask Launchpad is a bot like the others, created in every workspace, with its own memory", "It reads Launchpad state and answers with proposals through the Launchpad CLI tool", "Manual mode waits for your click; auto mode (bot.toml) starts one-time proposals; Edit sends a proposal to the composer", "Its BOT.md and bot.toml have versions; edit them, or replace it with your own bot", "A bot is an agent that stays: one thread shown as a chat, its instructions in BOT.md, its notes in its own memory; the trace stays in Agents", "In a bot's chat, proposals are one line with Details, handled ones fold into a receipt, and the message box answers its questions", "Routines run a bot on their own: on a schedule, on GitHub activity, when another agent is done or on a new message in a channel; quiet runs stay silent", "Bots talk to each other in channels and DMs you can read and join; the loop guard stops ping-pong and Allow 10 more lets it go on"],
  setup: () => useWorkspace.getState().switchTo(WS.acme),
  steps: [
    {
      id: "ask.field", route: "/", anchor: "topbar.field",
      title: "Ask from anywhere",
      body: "The top-bar field talks to a bot of the workspace: Ask Launchpad unless you pick another one. The page you are on goes with your message, so “review this” knows which pull request you mean; the ✕ leaves it out. Enter sends with Ask, the green button; Launch starts an agent with the same text.",
      tryIt: { text: "press Ask.", done: () => useAskPanel.getState().open, doIt: () => useAskPanel.getState().show(ASK_BOT) },
    },
    {
      id: "ask.panel", route: "/", anchor: "ask.panel",
      title: "The chat beside the page",
      body: "Ask opens the bot's chat as a panel: the same chat as its page. Its answers end in one-line cards: proposals you start and ready clicks for what stays yours, each with Details; changes it made directly are receipts on the steps line.",
      enter: () => useAskPanel.getState().show(ASK_BOT),
    },
    {
      id: "ask.ready", route: ASK_ROUTE, anchor: "proposal.card", anchorId: READY,
      title: "Your click",
      body: "Merging, publishing, posting, sending and marking Inbox threads done stay yours. A bot can prepare them as a ready click: the card runs the same call as the button on its page, with the same confirmation, and only when you click it.",
      enter: () => useAskPanel.getState().close(),
    },
    {
      id: "ask.changed", route: ASK_ROUTE, anchor: "chat.changed",
      title: "Changed",
      body: "With admin access, Ask Launchpad changes Launchpad's own records directly: bots, routines, schedules, folder settings and presets. Each change is a small receipt on the steps line of its turn, like “changed folder storefront”, and opens the record. A tap on the line shows each change with the value before and after.",
    },
    {
      id: "ask.thread", route: ASK_ROUTE, anchor: "bots.thread",
      title: "Ask Launchpad",
      body: "Every workspace starts with this bot. Its page is a chat: your messages, its answers and the cards it writes. One quiet line per turn counts the steps behind an answer and the time it took; a tap lists the step titles. The full trace stays in Agents (⋯ → Open current session in Agents). It reads your folders, agents, inbox, schedules and quotas with the Launchpad CLI, and keeps what it learns about you in its own memory.",
    },
    {
      id: "ask.question", route: ASK_ROUTE, anchor: "bots.reply",
      title: "Ask in plain words",
      body: "Ask what needs you, or describe work. The chips over the message box are its starters (starters in bot.toml): one sends at once, one ending in “…” fills the box for you to finish. It answers in the chat and writes proposal cards for what it suggests; they appear at the turn that wrote them. A question it asks shows its options as chips; to answer in your own words, type in the same message box.",
      tryIt: { text: "ask “What needs me today?”.", done: () => proposal()?.state === "proposed", doIt: () => api.post(`/api/sessions/${THREAD}/input`, { text: "What needs me today?" }) },
    },
    {
      id: "ask.proposal", route: ASK_ROUTE, anchor: "proposal.card", anchorId: TODAY_PROPOSAL,
      title: "Proposals wait for you",
      body: "In the chat, a proposal is one line: its title, its button, and what it does in plain words. Start launches it; its ⋯ holds Details (the full card: profile, folder, diffs), Dismiss and Edit (opens it in the composer). Paid usage is always off for proposals. Proposals of every bot also wait in Today, under Approve, as full cards.",
      tryIt: { text: "press Start.", done: () => proposal()?.state === "started", doIt: () => api.post(`/api/proposals/${TODAY_PROPOSAL}/accept`) },
    },
    {
      id: "ask.started", route: ASK_ROUTE, anchor: "proposal.card", anchorId: TODAY_PROPOSAL,
      title: "A normal agent",
      body: "Once handled, a card folds into a one-line receipt: “Started …”, “Added routine …” or “Dismissed …”, linking to what it made. A started proposal is a normal agent, listed in Agents like any other.",
    },
    {
      id: "ask.customize", route: `${ASK_ROUTE}?panel=about`, anchor: "bots.files",
      title: "Make it yours",
      body: "About holds what defines the bot. Its BOT.md says how it works and answers; edit it here, or tell it “from now on, …” and it proposes the change. An edit reaches its current session with your next message. Access sums up its profile, model, folder, permissions and proposal mode in one line; Edit changes them, and ⋯ → Edit as file opens bot.toml. You can also delete it and build your own: any bot with the Launchpad CLI tool can do the same.",
    },
    {
      id: "ask.bots", route: "/bots", anchor: "sidebar.bots",
      title: "Bots: agents that stay",
      body: "Your other bots each have a job, one thread you come back to, and their own memory: notes they keep between sessions. Ask Launchpad can propose one for you.",
    },
    {
      id: "ask.channels", route: "/bots", anchor: "sidebar.channels",
      title: "Channels and bot DMs",
      body: "Under the bots, Channels and Bot DMs list where bots talk to each other. A bot writes there only with launchpad channel post or launchpad dm; its own answers stay in its chat. A dot means new messages. When a bot asks you (launchpad channel post --ask-owner) or mentions @owner, the channel gets an amber count and one Today row, \"Release asks you in #release-train\", whose Reply answers in the channel; other channel messages never go into Today.",
    },
    {
      id: "ask.channel", route: CHANNEL_ROUTE, anchor: "channel.messages",
      title: "A channel page",
      body: "Every message shows who woke up and why under it (a mention, a subscription, a DM), with a link to that bot's chat, and who was busy or skipped. Your own message wakes the bots you mention with @ (and in a DM, both bots), and Pause in the header stops every delivery at once.",
    },
    {
      id: "ask.channel-guard", route: CHANNEL_ROUTE, anchor: "channel.guard",
      title: "The loop guard",
      body: "Each bot message counts one hop more than the message that woke its author. Past the channel's hop limit (3) or 30 bot messages in an hour, deliveries are skipped and this line turns amber with the limit that stopped them. Allow 10 more lets the conversation go on; your next message resets both counts.",
    },
    {
      id: "ask.channel-members", route: `${CHANNEL_ROUTE}?panel=members`, anchor: "channel.members",
      title: "Members",
      body: "Members lists who reads and posts here: you, always, then each member bot with when it is woken (every message, when mentioned, a daily digest at 08:00 listing what is new, or muted). Add a bot of the workspace or remove one; an admin bot can do the same, and its change shows in its chat. A bot that is not a member asks to join with a card you accept, and a bot without admin access proposes a new channel the same way. New channel on the Bots page creates one; archived channels wait under Bots → Archived channels, where you bring them back.",
    },
    {
      id: "ask.bot-thread", route: BOT_ROUTE, anchor: "bots.thread",
      title: "One thread, many sessions",
      body: "The chat shows the whole thread, with a divider at each new session; older messages load as you scroll up. Messages go to the current session. A new session starts when its profile, folder, model, effort or permission mode changes (the divider says why), or when you pick Start a new session in ⋯, with a reason the divider shows; it reads the previous trace only when it needs it. After you edit BOT.md, “Instructions updated” over the message box means your next message brings the new one, in the same session. A routine run with nothing to report is one quiet line.",
      tryIt: { text: "ask the Maintainer for the refund test.", done: () => botSessions().some((s) => (world.events[s.id] ?? []).some((e) => (e.body.kind === "user_message" && e.body.text.toLowerCase().includes("refund")) || e.body.kind === "ask_answered")), doIt: () => api.post(`/api/bots/${BOT}/message`, { text: "Prepare the refund end-to-end test as a Local PR." }) },
    },
    {
      id: "ask.bot-answer", route: BOT_ROUTE, anchor: "chat.answering",
      title: "One message box",
      body: "When the bot asks you something, its options are chips on the question, and the message box answers it: “Answering: …” over the box says so. Cancel sends a normal message instead and closes the question as skipped. A report that waits for your answer works the same way with its Reply. Approval requests keep their Allow and Deny buttons, and typed text never approves anything.",
    },
    {
      id: "ask.bot-from", route: BOT_ROUTE, anchor: "chat.from",
      title: "Messages from other bots",
      body: "A message from another bot or agent (an admin bot's message, or a card you accepted) shows its sender instead of your bubble. The bot reads it under a line saying it is not from you, so it treats it as information, not as your instructions. After three automatic messages in a row, Launchpad stops the chain until you write again.",
    },
    {
      id: "ask.bot-waiting", route: BOT_ROUTE, anchor: "chat.queued",
      title: "Messages that wait",
      body: "A hook or Telegram message that arrives while the bot works waits for the end of its turn, as a grey row at the bottom of the chat. Cancel drops it before it is sent.",
    },
    {
      id: "ask.bot-about", route: BOT_ROUTE, anchor: "bots.about",
      title: "About the bot",
      body: "The chat is the page; the line under the name says what the bot does now, or when it runs next. About opens a drawer with what defines it, top to bottom: BOT.md, Triggers, Memory (its own notes, loaded at the start of each session, up to 100 lines), Access and Versions (every version of its files for 30 days, with Restore). This device remembers whether it was open. ⋯ holds Pause all triggers, Start a new session, Open current session in Agents, Edit as file and Delete. On the phone, ⓘ opens About as a page.",
    },
    {
      id: "ask.bot-routines", route: `${BOT_ROUTE}?panel=about`, anchor: "bots.triggers",
      title: "Triggers: work on its own",
      body: "A routine sends the bot a task by itself: every weekday morning, when CI fails on GitHub, when another agent is done, or when a new message arrives in a channel (never for its own posts or for messages the channel already delivered to it). Each run lands in its thread and ends with a report, or with No update, which never notifies you. “When an agent is done” leaves bots' answers out unless you check Bots' answers count too, and a routine started by events runs at most six times an hour. Incoming hooks and the Telegram command that message the bot are listed too. Each routine is a file of the bot (Edit as file in its ⋯), and a bot proposes changes to its BOT.md and routines as one card with a diff per file. Pause all stops every trigger of the bot; your own messages still go through.",
      tryIt: { text: "press Run now on a routine.", done: () => (world.crons.find((c) => c.id === DAILY)?.history.length ?? 0) > dailyRuns, doIt: () => api.post(`/api/launch/crons/${DAILY}/run`) },
      enter: () => { dailyRuns = world.crons.find((c) => c.id === DAILY)?.history.length ?? 0; },
    },
    {
      id: "ask.bot-start", route: `${BOT_ROUTE}?panel=about`, anchor: "bots.start-preview",
      title: "What it sees at start",
      body: "Shows the instructions and the first prompt of its next session, in parts you open one by one: the knowledge base and its INDEX.md, its memory, the Launchpad CLI reference, BOT.md and your message. Launchpad builds them like a real start. The composer has the same button for any launch.",
    },
  ],
};
