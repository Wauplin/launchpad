// Chapter 6: Ask Launchpad, proposals and the owner's click; then bots, agents that stay.

import type { Chapter } from "../tour/types";
import { api } from "@/lib/api";
import { useWorkspace } from "@/stores/workspace";
import { world } from "@/demo/world";
import { WS } from "@/demo/world/ids";
import { TODAY_PROPOSAL } from "@/demo/scenarios/askToday";
import { meta } from "./meta";
import { BOT } from "@/demo/world/bots";

const THREAD = "demo-s-ask-acme";
const proposal = () => world.proposals.find((p) => p.id === TODAY_PROPOSAL);
const BOT_ROUTE = `/bots/${BOT}`;
const DAILY = "demo-routine-daily";
let dailyRuns = 0;
const botSessions = () => world.sessions.filter((s) => s.launch_spec.bot?.id === BOT);

export const ask: Chapter = {
  ...meta("ask"),
  learned: ["Ask Launchpad keeps one thread per workspace and reads its state", "It answers with proposals: launches, messages, schedules and crons", "Manual mode waits for your click; auto mode starts new proposals; Edit sends a proposal to the composer", "A bot is an agent that stays: one thread, its instructions in BOT.md, its notes in the knowledge base", "Routines run a bot on their own: on a schedule, on GitHub activity or when another agent is done; quiet runs stay silent"],
  setup: () => useWorkspace.getState().switchTo(WS.acme),
  steps: [
    {
      id: "ask.thread", route: "/ask", anchor: "ask.chat",
      title: "Ask Launchpad",
      body: "One thread per workspace. The assistant reads your folders, agents, inbox, schedules and quotas with read-only commands. It cannot start, post or change anything.",
    },
    {
      id: "ask.question", route: "/ask", anchor: "session.reply",
      title: "Ask in plain words",
      body: "Ask what needs you, or describe work. The assistant answers in the thread and writes proposal cards for what it suggests. Use Presets for reusable instructions; choose Ask Launchpad availability in Settings → Presets.",
      tryIt: { text: "ask “What needs me today?”.", done: () => proposal()?.state === "proposed", doIt: () => api.post(`/api/sessions/${THREAD}/input`, { text: "What needs me today?" }) },
    },
    {
      id: "ask.proposal", route: "/ask", anchor: "proposal.card", anchorId: TODAY_PROPOSAL,
      title: "Proposals wait for you",
      body: "Each card shows the profile, the folder and anything unusual. Start launches it, Edit opens it in the composer, Dismiss drops it. Paid usage is always off for proposals.",
      tryIt: { text: "press Start.", done: () => proposal()?.state === "started", doIt: () => api.post(`/api/proposals/${TODAY_PROPOSAL}/accept`) },
    },
    {
      id: "ask.started", route: "/ask", anchor: "proposal.card", anchorId: TODAY_PROPOSAL,
      title: "A normal agent",
      body: "A started proposal becomes a normal agent, listed in Agents like any other. Cron proposals say “Creates a repeating agent” before you accept them.",
    },
    {
      id: "ask.bots", route: "/bots", anchor: "sidebar.bots",
      title: "Bots: agents that stay",
      body: "Ask Launchpad is the built-in bot. Your own bots each have a job, one thread you come back to, and notes they keep in the knowledge base. Ask Launchpad can propose one for you.",
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
      body: "Memory lists its notes in the knowledge base. Instructions holds BOT.md and bot.toml, with every version for 30 days. Settings shows its profile, folder, permissions and what it can reach.",
    },
  ],
};
