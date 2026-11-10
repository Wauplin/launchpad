// Bots part of the demo world: Ask Launchpad (created with the workspace) and the Maintainer of storefront in
// Acme, with their files and versions. Their sessions are in world/agents.ts (`demo-s-ask-acme`,
// `demo-s-bot-maintainer`), the Maintainer's notes in world/knowledge.ts (bots/maintainer/).

import type { Bot } from "@/gen/Bot";
import type { Change } from "@/gen/Change";
import type { Activity } from "@/gen/Activity";
import type { QueuedMessage } from "@/gen/QueuedMessage";
import { clock } from "../daemon";
import { P, WS } from "./ids";

export const BOT = "demo-bot-maintainer";
export const ASK_BOT = "demo-bot-ask-acme";

export const ASK_DEFAULT = "# Ask Launchpad\n\nYou help the owner run their day in Launchpad: answer questions about their agents, folders, Inbox and quotas, and turn requests into proposals they start with one click.\n\n## How you work\n\n- Read the state you need with the Launchpad CLI before you answer or propose. Check facts instead of guessing.\n- You do not do the work yourself: you propose it. One proposal per independent task.\n- When the owner asks you to change Launchpad itself (a bot, its instructions or settings, a routine, a schedule, a workspace), make the change directly with your admin access, then say what you changed. Starting agents and messaging them stay proposals unless the owner says otherwise. To steer an agent that already works on it, propose a message to its session. To answer \"what needs me\", propose a status card.\n- Work that should wait for quota: a schedule with a quota condition.\n- A recurring helper with its own memory: propose a bot. A task for an existing bot: a message to its current session. Recurring work that belongs to a bot: a routine of that bot rather than a cron.\n- To refine a card that still waits, replace it instead of adding a second one, and say which card you replaced.\n- Note only durable things about the owner (their habits, how they like proposals) or what they ask you to remember, in a few short lines. Never log your answers, proposals or runs.\n\n## How you answer\n\n- Short sentences, plain words. Lead with the answer.\n- After proposing, say what each card does and check its returned state.\n";

/** Ask Launchpad's starters, as Home seeds them (`ask_bot::STARTERS`). */
export const ASK_STARTERS = ["What needs me?", "How are my agents doing?", "What happened today?", "Start an agent to …", "What is the status of …"];

export const settingsText = (b: Bot["settings"]) => `# Edited from the bot's Settings tab. folder and profile are IDs; see Folders and Settings → Profiles.\n# Without folder, each session starts in a temporary folder.\n# proposals: "manual" (every proposal waits for your click) or "auto" (one-time starts apply at once).\n# launchpad: what it may do with the Launchpad CLI: "read", "propose" or "admin" (change Launchpad's settings).\n# inbox_done = true: it may mark your Inbox threads done (or unsubscribe from them) itself, here and on GitHub.\n# starters: ready-made messages over the message box; one ending in \"…\" fills the box so you finish it.\nname = "${b.name}"\n${b.profile ? `profile = "${b.profile}"\n` : ""}${b.model ? `model = "${b.model}"\n` : ""}${b.folder ? `folder = "${b.folder}"\n` : ""}permission_mode = "${b.permission_mode}"\nproposals = "${b.proposals ?? "manual"}"\nlaunchpad = "${b.launchpad ?? "propose"}"\n${b.inbox_done ? "inbox_done = true\n" : ""}${b.starters?.length ? `starters = [${b.starters.map((x) => JSON.stringify(x)).join(", ")}]\n` : ""}`;

export function botsWorld() {
  const settings: Bot["settings"] = { name: "Maintainer", profile: "claude-max", folder: P.storefront, permission_mode: "yolo", proposals: "manual", launchpad: "propose" };
  const ask: Bot["settings"] = { name: "Ask Launchpad", permission_mode: "yolo", proposals: "manual", launchpad: "admin", starters: [...ASK_STARTERS] };
  const bots: Bot[] = [{
    id: ASK_BOT, workspace_id: WS.acme, slug: "ask-launchpad", created_at: clock.ago(60 * 24 * 30), settings: ask, instructions: ASK_DEFAULT,
    settings_text: settingsText(ask), settings_error: null, own_folder: true, paused: false, version: 2,
  }, {
    id: BOT, workspace_id: WS.acme, slug: "maintainer", created_at: clock.ago(60 * 26), settings,
    instructions: "# Maintainer\n\nMaintain storefront: answer my questions about the code, investigate issues and failing checks, and prepare fixes as Local PRs for my review. Keep answers short and cite commits.\n",
    settings_text: settingsText(settings), settings_error: null, own_folder: false, paused: false, version: 3,
  }];
  const you = { kind: "owner" as const, name: "You", session_id: null, model: null, runner: null };
  const botChanges: Record<string, (Change & { patch: string })[]> = {
    [BOT]: [
      { id: 3, path: "BOT.md", sha256: "b1", new_sha256: "b2", at: clock.ago(60 * 25), session_id: null, author: you, added: 1, removed: 1, patch: "@@ -1,3 +1,3 @@\n # Maintainer\n \n-Maintain storefront: answer my questions about the code and prepare fixes as Local PRs.\n+Maintain storefront: answer my questions about the code, investigate issues and failing checks, and prepare fixes as Local PRs for my review. Keep answers short and cite commits.\n" },
      { id: 2, path: "bot.toml", sha256: null, new_sha256: "t1", at: clock.ago(60 * 26), session_id: null, author: you, added: 5, removed: 0, patch: `@@ -0,0 +1,5 @@\n${settingsText(settings).trimEnd().split("\n").map((l) => `+${l}`).join("\n")}\n` },
      { id: 1, path: "BOT.md", sha256: null, new_sha256: "b1", at: clock.ago(60 * 26), session_id: null, author: you, added: 3, removed: 0, patch: "@@ -0,0 +1,3 @@\n+# Maintainer\n+\n+Maintain storefront: answer my questions about the code and prepare fixes as Local PRs.\n" },
    ],
  };
  // What Ask Launchpad changed directly (admin access): a Changed line in its chat.
  const activity: Activity[] = [
    { id: "demo-act-storefront", session_id: "demo-s-ask-acme", workspace_id: WS.acme, action: "folder.set", args: [P.storefront], target: "storefront",
      body: { when_done: "keep" }, before: { when_done: "merge" }, undo: { action: "folder.set", args: [P.storefront], body: { when_done: "merge" } }, undone_at: null, created_at: clock.ago(13) },
  ];
  // A hook message waiting for the Maintainer's turn to end (a grey row at the bottom of its chat, with Cancel).
  const botQueued: Record<string, QueuedMessage[]> = {
    [BOT]: [{ n: 1, at: clock.ago(2), source: "hook", text: "The incoming hook \"CI failures\" received a request: { \"workflow\": \"checkout-e2e\", \"conclusion\": \"failure\" }" }],
  };
  return { bots, botChanges, activity, botQueued };
}
