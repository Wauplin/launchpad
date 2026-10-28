// Chapter 2: the composer, launching an agent and watching its prerecorded run.

import type { Chapter } from "../tour/types";
import { emptySpec, useComposer } from "@/stores/composer";
import { world } from "@/demo/world";
import { P } from "@/demo/world/ids";
import { isPlaying, skipToEnd } from "@/demo/player";
import { anchor, clickAnchor, navigate, waitFor } from "./helpers";
import { meta } from "./meta";

export const EXAMPLE_PROMPT = "Fix the flaky checkout test in storefront: it fails about one CI run in ten.";

let before = 0;
/** The session this chapter launched, once it exists. */
const launched = () => world.sessions.length > before ? world.sessions.find((s) => s.prompt.startsWith(EXAMPLE_PROMPT.slice(0, 20)) && s.started_at > 0 && !s.id.startsWith("demo-s-")) ?? null : null;

export const launch: Chapter = {
  ...meta("launch"),
  learned: ["Launch opens the composer, prompt first", "A new worktree keeps your checkout untouched", "The option line shows the profile and only what differs from the defaults", "Trace shows the run; Summary its outcome and next steps"],
  setup: () => { before = world.sessions.length; },
  steps: [
    {
      id: "launch.open", route: "/agents", anchor: "topbar.launch",
      title: "Launch an agent",
      body: "Launch opens the composer, where you describe the task. A draft survives reloads; Minimize keeps it as a badge on this button.",
      tryIt: { text: "click Launch.", done: () => { const c = useComposer.getState(); return c.open && !c.minimized; }, doIt: () => clickAnchor("topbar.launch") },
    },
    {
      id: "launch.prompt", anchor: "composer.prompt",
      title: "Prompt first",
      body: "Write the task like you would for a colleague. @ picks a folder, # picks an issue or PR as context. Agents also get the instruction to end with a short summary.",
      tryIt: {
        text: "write a prompt, or use the example.",
        done: () => useComposer.getState().spec.text.trim().length > 10,
        doIt: () => useComposer.getState().update((s) => ({ ...s, text: EXAMPLE_PROMPT, harness: "claude", profile: "claude-max", model: null, effort: null, target: { kind: "project", project_id: P.storefront }, worktree: s.worktree ?? emptySpec().worktree })),
      },
    },
    {
      id: "launch.where", anchor: "composer.runin",
      title: "Folder and Run in",
      body: "The agent works in a folder. In a Git folder it gets its own worktree: a new branch and folder, so your checkout stays untouched. When done chooses between keeping the branch for review and merging it.",
      enter: () => { const s = useComposer.getState().spec; if (s.target.kind === "none") useComposer.getState().update((x) => ({ ...x, target: { kind: "project", project_id: P.storefront } })); },
    },
    {
      id: "launch.options", anchor: "composer.options",
      title: "Profile and options",
      body: "This line shows the profile (which agent, and how it is paid), then only what differs from the defaults: permission mode, presets, paid usage. Paid usage after the included quota is off unless you turn it on.",
    },
    {
      id: "launch.go", anchor: "composer.launch",
      title: "Launch",
      body: "Launch now starts at once. The arrow offers Start later (time, after other agents, when quota is back) and Repeat on a schedule. In the demo, the run replays a prerecorded one.",
      tryIt: {
        text: "click Launch.",
        done: () => !!launched(),
        doIt: async () => { (await waitFor(() => anchor("composer.launch")?.querySelector("button")))?.click(); },
      },
    },
    {
      id: "launch.watch", anchor: "session.views",
      title: "Watch it work",
      body: "Trace shows each step as it happens: reasoning, commands, file changes. When the turn ends, Summary shows the outcome, the checks run and fact-based next steps.",
      enter: async () => { const s = await waitFor(launched); if (s) void navigate(`/agents?session=${s.id}`); },
      run: () => launched()?.id ?? null,
      tryIt: { text: "wait for the run to finish (or Skip to end).", done: () => { const s = launched(); return !!s && !isPlaying(s.id) && s.status === "waiting"; }, doIt: () => { const s = launched(); if (s) skipToEnd(s.id); } },
    },
  ],
};
