// Bots part of the demo world: the Maintainer of storefront in Acme, with its files and their versions. Its
// session is in world/agents.ts (`demo-s-bot-maintainer`), its notes in world/knowledge.ts (bots/maintainer/).

import type { Bot } from "@/gen/Bot";
import type { Change } from "@/gen/Change";
import { clock } from "../daemon";
import { P, WS } from "./ids";

export const BOT = "demo-bot-maintainer";

export const settingsText = (b: Bot["settings"]) => `# Edited from the bot's Settings tab. folder and profile are IDs; see Folders and Settings → Profiles.\nname = "${b.name}"\n${b.profile ? `profile = "${b.profile}"\n` : ""}${b.model ? `model = "${b.model}"\n` : ""}folder = "${b.folder}"\npermission_mode = "${b.permission_mode}"\n`;

export function botsWorld() {
  const settings: Bot["settings"] = { name: "Maintainer", profile: "claude-max", folder: P.storefront, permission_mode: "yolo" };
  const bots: Bot[] = [{
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
