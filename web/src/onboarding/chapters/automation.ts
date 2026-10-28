// Chapter 9: Start later, crons, quota protection and scheduled follow-ups.

import type { Chapter } from "../tour/types";
import { useComposer } from "@/stores/composer";
import { useWorkspace } from "@/stores/workspace";
import { world } from "@/demo/world";
import { clock } from "@/demo/daemon";
import { continueQuota } from "@/demo/player";
import { meta } from "./meta";

const PAUSED = "demo-s-quota";
const paused = () => world.sessions.find((s) => s.id === PAUSED)?.status === "quota_paused";

/** Moves the demo clock past the quota reset of the paused agent, then lets it continue. */
function fastForward() {
  const resets = world.sessions.find((s) => s.id === PAUSED)?.quota_pause?.blockers.map((b) => (b.resets_at ?? 0) * 1000) ?? [];
  const until = Math.max(0, ...resets);
  if (until > clock.now()) clock.fastForward(until - clock.now() + 60_000);
  continueQuota(PAUSED);
}

export const automation: Chapter = {
  ...meta("automation"),
  learned: ["Start later waits for a time, other agents or quota; all conditions must be met", "Repeat on a schedule creates a cron agent", "Quota protection pauses agents before paid usage, and continues them when quota is back", "Send later delivers a message to an agent later"],
  setup: () => useWorkspace.getState().switchTo(null),
  steps: [
    {
      id: "automation.later", route: "/agents", anchor: "composer.launch",
      title: "Start later",
      body: "The arrow next to Launch offers Start later: after a delay, at a time, after other agents are Done, or when quota is back. A launch can combine conditions; all must be met.",
      enter: async () => { await new Promise((r) => setTimeout(r, 400)); const c = useComposer.getState(); if (!c.open) c.start(); else if (c.minimized) c.setMinimized(false); },
    },
    {
      id: "automation.cron", route: "/agents", anchor: "agents.group", anchorId: "Cron",
      title: "Repeat on a schedule",
      body: "Repeat on a schedule creates a cron agent: each run is a normal agent. Runs that are more than an hour late are skipped, never caught up.",
      enter: () => useComposer.getState().setMinimized(true),
    },
    {
      id: "automation.quota", route: `/agents?session=${PAUSED}`, anchor: "session.primary",
      title: "Quota protection",
      body: "When a subscription's included quota runs out, protected agents pause instead of using paid credits. Paid usage stays off unless you allow it for a launch or a session.",
    },
    {
      id: "automation.continue", route: `/agents?session=${PAUSED}`, anchor: "statusbar.quota",
      title: "Back when quota is",
      body: "The agent continues by itself once a fresh reading confirms quota in both windows. It never guesses from the reset time alone.",
      tryIt: { text: "fast-forward the demo clock past the reset.", done: () => !paused(), doIt: fastForward },
    },
    {
      id: "automation.followup", route: `/agents?session=${PAUSED}`, anchor: "session.reply",
      title: "Scheduled launches and follow-ups",
      body: "The quota reset also started the scheduled launch that waited for it. Send later, from an agent's message box, delivers a message to that same agent at a time or when quota is back.",
    },
  ],
};
