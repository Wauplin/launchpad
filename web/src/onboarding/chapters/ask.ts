// Chapter 6: Ask Launchpad, a bot like the others: proposals and the owner's click; then bots, agents that stay.

import type { Chapter } from "../tour/types";
import { api } from "@/lib/api";
import { useWorkspace } from "@/stores/workspace";
import { world } from "@/demo/world";
import { WS } from "@/demo/world/ids";
import { TODAY_PROPOSAL } from "@/demo/scenarios/askToday";
import { meta } from "./meta";
import { ASK_BOT, BOT } from "@/demo/world/bots";

const THREAD = "demo-s-ask-acme";
const proposal = () => world.proposals.find((p) => p.id === TODAY_PROPOSAL);
const ASK_ROUTE = `/bots/${ASK_BOT}`;
const BOT_ROUTE = `/bots/${BOT}`;
const DAILY = "demo-routine-daily";
let dailyRuns = 0;
const botSessions = () => world.sessions.filter((s) => s.launch_spec.bot?.id === BOT);

export const ask: Chapter = {
  ...meta("ask"),
  learned: ["Ask Launchpad is a bot like the others, created in every workspace, with its notes in the knowledge base", "It reads Launchpad state and answers with proposals through the Launchpad CLI tool", "Manual mode waits for your click; auto mode (bot.toml) starts one-time proposals; Edit sends a proposal to the composer", "Its BOT.md and bot.toml have versions; edit them, or replace it with your own bot", "A bot is an agent that stays: one thread, its instructions in BOT.md, its notes in the knowledge base", "Routines run a bot on their own: on a schedule, on GitHub activity or when another agent is done; quiet runs stay silent"],
  setup: () => useWorkspace.getState().switchTo(WS.acme),
  steps: [
    {
      id: "ask.thread", route: ASK_ROUTE, anchor: "bots.thread",
      title: "Ask Launchpad",
      body: "Every workspace starts with this bot. It reads your folders, agents, inbox, schedules and quotas with the Launchpad CLI, and keeps what it learns about you in its notes in the knowledge base.",
    },
    {
      id: "ask.question", route: ASK_ROUTE, anchor: "session.reply",
      title: "Ask in plain words",
      body: "Ask what needs you, or describe work. It answers in its thread and writes proposal cards for what it suggests. Presets marked for bots appear here (Settings → Presets).",
      tryIt: { text: "ask “What needs me today?”.", done: () => proposal()?.state === "proposed", doIt: () => api.post(`/api/sessions/${THREAD}/input`, { text: "What needs me today?" }) },
    },
    {
      id: "ask.proposal", route: `${ASK_ROUTE}?panel=proposals`, anchor: "proposal.card", anchorId: TODAY_PROPOSAL,
      title: "Proposals wait for you",
      body: "Each card shows the profile, the folder and anything unusual. Start launches it, Edit opens it in the composer, Dismiss drops it. Paid usage is always off for proposals. Proposals of every bot also wait in Today, under Approve.",
      tryIt: { text: "press Start.", done: () => proposal()?.state === "started", doIt: () => api.post(`/api/proposals/${TODAY_PROPOSAL}/accept`) },
    },
    {
      id: "ask.started", route: `${ASK_ROUTE}?panel=proposals`, anchor: "proposal.card", anchorId: TODAY_PROPOSAL,
      title: "A normal agent",
      body: "A started proposal becomes a normal agent, listed in Agents like any other. Cron proposals say “Creates a repeating agent” before you accept them.",
    },
    {
      id: "ask.customize", route: `${ASK_ROUTE}?panel=settings`, anchor: "bots.files",
      title: "Make it yours",
      body: "Its BOT.md says how it works and answers; edit it here, or tell it “from now on, …” and it proposes the change. bot.toml holds its profile, folder, permissions and proposal mode. You can also delete it and build your own: any bot with the Launchpad CLI tool can do the same.",
    },
    {
      id: "ask.bots", route: "/bots", anchor: "sidebar.bots",
      title: "Bots: agents that stay",
      body: "Your other bots each have a job, one thread you come back to, and notes they keep in the knowledge base. Ask Launchpad can propose one for you.",
    },
    {
      id: "ask.bot-thread", route: BOT_ROUTE, anchor: "bots.thread",
      title: "One thread, many sessions",
      body: "Messages go to the bot's current session. A new session starts when its profile or folder changes, or on New session; it reads the previous trace only when it needs it.",
      tryIt: { text: "ask the Maintainer for the refund test.", done: () => botSessions().some((s) => (world.events[s.id] ?? []).some((e) => e.body.kind === "user_message" && e.body.text.toLowerCase().includes("refund"))), doIt: () => api.post(`/api/bots/${BOT}/message`, { text: "Prepare the refund end-to-end test as a Local PR." }) },
    },
    {
      id: "ask.bot-routines", route: `${BOT_ROUTE}?panel=routines`, anchor: "bots.routines",
      title: "Routines: work on its own",
      body: "A routine sends the bot a task by itself: every weekday morning, when CI fails on GitHub, or when another agent is done. Each run lands in its thread and ends with a report, or with No update, which never notifies you. Pause stops every routine of the bot.",
      tryIt: { text: "press Run now on a routine.", done: () => (world.crons.find((c) => c.id === DAILY)?.history.length ?? 0) > dailyRuns, doIt: () => api.post(`/api/launch/crons/${DAILY}/run`) },
      enter: () => { dailyRuns = world.crons.find((c) => c.id === DAILY)?.history.length ?? 0; },
    },
    {
      id: "ask.bot-panels", route: BOT_ROUTE, anchor: "bots.panels",
      title: "Memory, instructions and settings",
      body: "Memory lists its notes in the knowledge base and the sessions of its thread. Settings holds BOT.md, its profile, folder and permissions, what it can reach, and bot.toml, with every version for 30 days.",
    },
  ],
};
