// Bots part of the demo world: Ask Launchpad (created with the workspace) and the Maintainer of storefront in
// Acme, with their files and versions. Their sessions are in world/agents.ts (`demo-s-ask-acme`,
// `demo-s-bot-maintainer`), the Maintainer's notes in world/knowledge.ts (bots/maintainer/).

import type { Bot } from "@/gen/Bot";
import type { Change } from "@/gen/Change";
import { clock } from "../daemon";
import { P, WS } from "./ids";

export const BOT = "demo-bot-maintainer";
export const ASK_BOT = "demo-bot-ask-acme";

export const ASK_DEFAULT = "# Ask Launchpad\n\nYou help the owner run their day in Launchpad: answer questions about their agents, folders, Inbox and quotas, and turn requests into proposals they start with one click.\n\n## How you work\n\n- Read the state you need with the Launchpad CLI before you answer or propose. Check facts instead of guessing.\n- You do not do the work yourself: you propose it. One proposal per independent task.\n- When the owner asks you to change Launchpad itself (a bot, its instructions or settings, a routine, a schedule, a workspace), make the change directly with your admin access, then say what you changed. Starting agents and messaging them stay proposals unless the owner says otherwise. To steer an agent that already works on it, propose a message to its session. To answer \"what needs me\", propose a status card.\n- Work that should wait for quota: a schedule with a quota condition.\n- A recurring helper with its own memory: propose a bot. A task for an existing bot: a message to its current session. Recurring work that belongs to a bot: a routine of that bot rather than a cron.\n- To refine a card that still waits, replace it instead of adding a second one, and say which card you replaced.\n- Keep what you learn about the owner (their folders, habits, how they like proposals) in your notes, so the next session knows it.\n\n## How you answer\n\n- Short sentences, plain words. Lead with the answer.\n- After proposing, say what each card does and check its returned state.\n";

export const settingsText = (b: Bot["settings"]) => `# Edited from the bot's Settings tab. folder and profile are IDs; see Folders and Settings → Profiles.\n# proposals: "manual" (every proposal waits for your click) or "auto" (one-time starts apply at once).\n# launchpad: what it may do with the Launchpad CLI: "read", "propose" or "admin" (change Launchpad's settings).\nname = "${b.name}"\n${b.profile ? `profile = "${b.profile}"\n` : ""}${b.model ? `model = "${b.model}"\n` : ""}folder = "${b.folder}"\npermission_mode = "${b.permission_mode}"\nproposals = "${b.proposals ?? "manual"}"\nlaunchpad = "${b.launchpad ?? "propose"}"\n`;

export function botsWorld() {
  const settings: Bot["settings"] = { name: "Maintainer", profile: "claude-max", folder: P.storefront, permission_mode: "yolo", proposals: "manual", launchpad: "propose" };
  const ask: Bot["settings"] = { name: "Ask Launchpad", folder: P.ask, permission_mode: "yolo", proposals: "manual", launchpad: "admin" };
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
  return { bots, botChanges };
}
