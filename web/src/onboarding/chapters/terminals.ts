// Chapter 7: shells, the terminal dock and the harness TUIs, on the agent's runner and shown through Home.

import type { Chapter } from "../tour/types";
import { world } from "@/demo/world";
import { clickAnchor } from "./helpers";
import { meta } from "./meta";

let before = 0;

export const terminals: Chapter = {
  ...meta("terminals"),
  learned: ["The Terminal button opens a shell in the agent's folder, on its runner", "The dock keeps terminals open across pages", "Continue in the harness's own TUI, then Re-attach in Launchpad"],
  setup: () => { before = world.terminals.length; },
  steps: [
    {
      id: "terminals.menu", route: "/agents?session=demo-s-checkout", anchor: "terminal.menu",
      title: "Open a shell",
      body: "Terminal opens a shell in the folder of what you look at: here, the agent's worktree, on the runner of the agent. Home shows it here and passes your keys to it. The arrow lists the checkout and the other worktrees of the folder.",
      tryIt: { text: "click Terminal.", done: () => world.terminals.length > before, doIt: () => clickAnchor("terminal.menu") },
    },
    {
      id: "terminals.dock", anchor: "terminal.dock",
      title: "The terminal dock",
      body: "Terminals stay open while you move around Launchpad. Hide keeps them running; + New opens another one. In the demo, a recorded session replays and commands do not run.",
    },
    {
      id: "terminals.tui", route: "/agents?session=demo-s-cart", anchor: "session.primary",
      title: "Continue in the TUI",
      body: "⋯ → Continue in the harness's TUI opens Claude Code, Codex or OpenCode in a terminal on the agent's runner, on the same conversation. The agent shows In terminal until you press Re-attach.",
    },
    {
      id: "terminals.limits", anchor: "terminal.dock",
      title: "Good to know",
      body: "Terminals run on a runner, never on Home: closing the window keeps them, restarting that runner ends them. While a runner is offline, its Terminal buttons are off and say why. TUIs run outside quota protection.",
    },
  ],
};
