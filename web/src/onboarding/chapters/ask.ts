// Chapter 6: Ask Launchpad, proposals and the owner's click.

import type { Chapter } from "../tour/types";
import { api } from "@/lib/api";
import { useWorkspace } from "@/stores/workspace";
import { world } from "@/demo/world";
import { WS } from "@/demo/world/ids";
import { TODAY_PROPOSAL } from "@/demo/scenarios/askToday";
import { meta } from "./meta";

const THREAD = "demo-s-ask-acme";
const proposal = () => world.proposals.find((p) => p.id === TODAY_PROPOSAL);

export const ask: Chapter = {
  ...meta("ask"),
  learned: ["Ask Launchpad keeps one thread per workspace and reads its state", "It answers with proposals: launches, messages, schedules and crons", "Manual mode waits for your click; auto mode starts new proposals; Edit sends a proposal to the composer"],
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
      body: "Ask what needs you, or describe work. The assistant answers in the thread and writes proposal cards for what it suggests.",
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
  ],
};
