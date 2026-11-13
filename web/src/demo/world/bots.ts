// Bots part of the demo world: Ask Launchpad and the Librarian (created with the workspace) and the Maintainer of storefront in
// Acme, with their files and versions. Their sessions are in world/agents.ts (`demo-s-ask-acme`,
// `demo-s-bot-maintainer`). Their memory (`memory/` of each bot's folder) is `botMemory`; `Bot.memory` lists it.

import type { Bot } from "@/gen/Bot";
import type { Change } from "@/gen/Change";
import type { Activity } from "@/gen/Activity";
import type { QueuedMessage } from "@/gen/QueuedMessage";
import type { BotThread } from "@/gen/BotThread";
import { clock } from "../daemon";
import { P, WS } from "./ids";
import { botVersion, DAILY, MAINTAINER_BOT_MD, MAINTAINER_BOT_MD_PREV } from "../routine";

export const BOT = "demo-bot-maintainer";
/** The Maintainer's second thread: a review the owner started from the Chrome extension (world/agents.ts
 * `demo-s-bot-maintainer-120`), next to its main thread. */
export const PR_THREAD = "demo-thread-pr120";
export const ASK_BOT = "demo-bot-ask-acme";
export const LIBRARIAN_BOT = "demo-bot-librarian-acme";
/** Two more bots of Acme that talk with the Maintainer in its channels (world/channels.ts); they never ran here. */
export const RELEASE_BOT = "demo-bot-release";
export const DOCS_BOT = "demo-bot-docs";
const RELEASE_MD = "# Release\n\nShip storefront's weekly release: cut the branch on Monday, check with the Maintainer what goes in, tag once the owner agrees, and ask Docs for the changelog in #release-train.\n";
const DOCS_MD = "# Docs\n\nKeep the docs folder current: draft changelogs and how-tos when another bot asks in a channel, as Local PRs for the owner's review.\n";

export const ASK_DEFAULT = "# Ask Launchpad\n\nYou help the owner run their day in Launchpad: answer questions about their agents, folders, Inbox and quotas, and turn requests into proposals they start with one click.\n\n## How you work\n\n- Read the state you need with the Launchpad CLI before you answer or propose. Check facts instead of guessing.\n- You do not do the work yourself: you propose it. One proposal per independent task.\n- When the owner asks you to change Launchpad itself (a bot, its instructions or settings, a routine, a schedule, a workspace), make the change directly with your admin access, then say what you changed. Starting agents and messaging them stay proposals unless the owner says otherwise. To steer an agent that already works on it, propose a message to its session. To answer \"what needs me\", propose a status card.\n- Work that should wait for quota: a schedule with a quota condition.\n- A recurring helper with its own memory: propose a bot. A task for an existing bot: a message to its current session. Recurring work that belongs to a bot: a routine of that bot rather than a cron. A bot's BOT.md and routines are files of its folder (`routines/<name>.toml`): to change several of them, propose one `bot-files` card (one diff per file).\n- To refine a card that still waits, replace it instead of adding a second one, and say which card you replaced.\n- Note only durable things about the owner (their habits, how they like proposals) or what they ask you to remember, in a few short lines of your memory (`launchpad bot memory write`). Facts other agents need go in the knowledge base. Never log your answers, proposals or runs.\n\n## How you answer\n\n- Short sentences, plain words. Lead with the answer.\n- After proposing, say what each card does and check its returned state.\n";

/** Ask Launchpad's starters, as Home seeds them (`ask_bot::STARTERS`). */
export const ASK_STARTERS = ["What needs me?", "How are my agents doing?", "What happened today?", "Start an agent to …", "What is the status of …"];

/** The Librarian's BOT.md and starters, as Home seeds them (`librarian::DEFAULT_INSTRUCTIONS`, `librarian::STARTERS`). */
export const LIBRARIAN_DEFAULT = "# Librarian\n\nYou tend this workspace's knowledge base: the Markdown notes its agents share as their memory. Its folder is `$LAUNCHPAD_KB`; read AGENTS.md there before your first change. Keep it small, accurate and easy to search.\n\n## How you tidy\n\n- Merge duplicate notes. Prefer updating an existing note over adding a near duplicate.\n- Cut notes down to their durable findings: remove logs of sessions or runs, task status and results that only mattered once.\n- Add missing [[wikilinks]]. Mark outdated notes `status: superseded` with a link to the current one.\n- Merge every `*.concurrent-*` file into its note, then remove it.\n- Fold what is durable in `imported/` (memory files imported from a machine) into the notes, then remove those files.\n- Keep INDEX.md an accurate map, under 150 lines.\n- Change only what clearly improves the notes. When you are unsure whether a fact still holds, keep it and ask the owner.\n- Write only with `launchpad kb write|append|rm`. Every change keeps its author and can be restored for 30 days.\n\n## How the owner steers you\n\nThe owner talks to you in this chat. When they tell you how they want the notes kept (for example \"stop merging my how-tos\" or \"leave the meeting notes alone\"), follow it from then on: write it as one short line in your memory (`launchpad bot memory write`) and read your memory before each tidy. Your memory holds only these preferences, never a log of your runs.\n\n## How you answer\n\n- After a tidy, list what you changed in a few short lines, one per note. In a routine run with nothing to change, answer exactly `no_update`.\n- When the owner asks what you changed last time, answer from this thread (an earlier session's trace is named at its start).\n";
export const LIBRARIAN_STARTERS = ["Tidy the knowledge base now", "What did you change last time?", "From now on, …"];
/** Its weekly routine, created paused (`librarian::ROUTINE`). */
export const LIBRARIAN_TIDY = "Tidy the knowledge base as your instructions say, following the preferences in your memory. End with a short list of what you changed, or with exactly no_update when nothing needed a change.";

export const settingsText = (b: Bot["settings"]) => `# Edited from the bot's page (About → Access, or ⋯ → Edit as file). folder and profile are IDs; see Folders and Settings → Profiles.\n# Without folder, each session starts in a temporary folder.\n# proposals: "manual" (every proposal waits for your click) or "auto" (one-time starts apply at once).\n# launchpad: what it may do with the Launchpad CLI: "read", "propose" or "admin" (change Launchpad's settings).\n# inbox_done = true: it may mark your Inbox threads done (or unsubscribe from them) itself, here and on GitHub.\n# starters: ready-made messages over the message box; one ending in \"…\" fills the box so you finish it.\nname = "${b.name}"\n${b.profile ? `profile = "${b.profile}"\n` : ""}${b.model ? `model = "${b.model}"\n` : ""}${b.folder ? `folder = "${b.folder}"\n` : ""}permission_mode = "${b.permission_mode}"\nproposals = "${b.proposals ?? "manual"}"\nlaunchpad = "${b.launchpad ?? "propose"}"\n${b.inbox_done ? "inbox_done = true\n" : ""}${b.starters?.length ? `starters = [${b.starters.map((x) => JSON.stringify(x)).join(", ")}]\n` : ""}`;

const CHECKOUT_NOTE = "---\ntitle: Checkout, as the Maintainer sees it\ntype: project\nstatus: current\n---\n- Webhooks are verified before the order is saved (7f03b11).\n- The e2e test polls the order for 10 seconds (a41c9e2).\n- Risk: the refund path has no end-to-end test yet. The owner may ask for one.\n";

export function botsWorld() {
  const settings: Bot["settings"] = { name: "Maintainer", profile: "claude-max", folder: P.storefront, permission_mode: "yolo", proposals: "manual", launchpad: "propose" };
  const ask: Bot["settings"] = { name: "Ask Launchpad", permission_mode: "yolo", proposals: "manual", launchpad: "admin", starters: [...ASK_STARTERS] };
  const librarian: Bot["settings"] = { name: "Librarian", permission_mode: "yolo", proposals: "manual", launchpad: "propose", starters: [...LIBRARIAN_STARTERS] };
  const release: Bot["settings"] = { name: "Release", profile: "claude-max", permission_mode: "yolo", proposals: "manual", launchpad: "propose" };
  const docs: Bot["settings"] = { name: "Docs", profile: "codex-plus", folder: P.docs, permission_mode: "yolo", proposals: "manual", launchpad: "propose" };
  const bots: Bot[] = [{
    id: ASK_BOT, workspace_id: WS.acme, slug: "ask-launchpad", created_at: clock.ago(60 * 24 * 30), settings: ask, instructions: ASK_DEFAULT,
    settings_text: settingsText(ask), settings_error: null, own_folder: true, paused: false, version: 2, memory: [], routines: {},
    instructions_version: botVersion(ASK_DEFAULT), memory_loaded: { loaded: 0, total: 0 }, folder_unused: null,
  }, {
    // Seeded with the workspace too, its weekly tidy paused until you turn it on (world/agents.ts `demo-routine-tidy`).
    id: LIBRARIAN_BOT, workspace_id: WS.acme, slug: "librarian", created_at: clock.ago(60 * 24 * 30), settings: librarian, instructions: LIBRARIAN_DEFAULT,
    settings_text: settingsText(librarian), settings_error: null, own_folder: true, paused: false, version: 3, memory: [], routines: {},
    instructions_version: botVersion(LIBRARIAN_DEFAULT), memory_loaded: { loaded: 0, total: 0 }, folder_unused: null,
  }, {
    id: BOT, workspace_id: WS.acme, slug: "maintainer", created_at: clock.ago(60 * 26), settings,
    instructions: MAINTAINER_BOT_MD,
    settings_text: settingsText(settings), settings_error: null, own_folder: false, paused: false, version: 4, memory: [], routines: {},
    instructions_version: botVersion(MAINTAINER_BOT_MD), memory_loaded: { loaded: 0, total: 0 }, folder_unused: null,
  }, {
    id: RELEASE_BOT, workspace_id: WS.acme, slug: "release", created_at: clock.ago(60 * 24 * 6), settings: release, instructions: RELEASE_MD,
    settings_text: settingsText(release), settings_error: null, own_folder: false, paused: false, version: 1, memory: [], routines: {},
    instructions_version: botVersion(RELEASE_MD), memory_loaded: { loaded: 0, total: 0 }, folder_unused: null,
  }, {
    id: DOCS_BOT, workspace_id: WS.acme, slug: "docs", created_at: clock.ago(60 * 24 * 6), settings: docs, instructions: DOCS_MD,
    settings_text: settingsText(docs), settings_error: null, own_folder: false, paused: false, version: 1, memory: [], routines: {},
    instructions_version: botVersion(DOCS_MD), memory_loaded: { loaded: 0, total: 0 }, folder_unused: null,
  }];
  // Each bot's memory/ notes: path (in its folder) → content.
  const botMemory: Record<string, Record<string, string>> = {
    [ASK_BOT]: { "memory/owner.md": "- Prefers one proposal per task, with a short reason.\n- Wants status cards in the morning, not a summary in chat.\n" },
    [BOT]: { "memory/checkout.md": CHECKOUT_NOTE },
  };
  const you = { kind: "owner" as const, name: "You", session_id: null, model: null, runner: null };
  const maintainer = { kind: "agent" as const, name: "What changed in checkout this week", session_id: "demo-s-bot-maintainer", model: "demo-large", runner: "laptop" };
  const botChanges: Record<string, (Change & { patch: string })[]> = {
    [BOT]: [
      // You asked for short answers after its current session started: that session gets the new BOT.md with your next message.
      { id: 6, path: "BOT.md", sha256: "b2", new_sha256: "b3", at: clock.ago(30), session_id: null, author: you, added: 1, removed: 1, patch: `@@ -1,3 +1,3 @@\n # Maintainer\n \n-${MAINTAINER_BOT_MD_PREV.split("\n")[2]}\n+${MAINTAINER_BOT_MD.split("\n")[2]}\n` },
      // You moved its daily check from 8:00 to 9:00 (its routine file).
      { id: 5, path: "routines/daily-check.toml", sha256: "r1", new_sha256: "r2", at: clock.ago(40), session_id: null, author: you, added: 1, removed: 1, patch: `@@ -3,4 +3,4 @@\n title = "Daily check"\n-schedule = "0 8 * * 1-5"\n+schedule = "0 9 * * 1-5"\n instructions = """\n ${DAILY}"""\n` },
      { id: 4, path: "memory/checkout.md", sha256: null, new_sha256: "m1", at: clock.ago(66), session_id: "demo-s-bot-maintainer", author: maintainer, added: CHECKOUT_NOTE.split("\n").length - 1, removed: 0, patch: `@@ -0,0 +1,${CHECKOUT_NOTE.split("\n").length - 1} @@\n${CHECKOUT_NOTE.trimEnd().split("\n").map((l) => `+${l}`).join("\n")}\n` },
      { id: 3, path: "BOT.md", sha256: "b1", new_sha256: "b2", at: clock.ago(60 * 25), session_id: null, author: you, added: 1, removed: 1, patch: "@@ -1,3 +1,3 @@\n # Maintainer\n \n-Maintain storefront: answer my questions about the code and prepare fixes as Local PRs.\n+Maintain storefront: answer my questions about the code, investigate issues and failing checks, and prepare fixes as Local PRs for my review.\n" },
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
    [BOT]: [{ n: 1, at: clock.ago(2), source: "hook", text: "The incoming hook \"Nightly CI\" received a request: { \"workflow\": \"checkout-e2e\", \"conclusion\": \"failure\" }", thread: "main", slot: false }],
  };
  // Each bot's threads (Home's `bot_thread` rows): main, and the Maintainer's review of #120 from the extension.
  const thread = (bot: string, id: string, title: string, key: string | null, minutes: number): BotThread => ({ bot_id: bot, id, key, title, created_at: clock.ago(minutes), archived_at: null });
  const botThreads: Record<string, BotThread[]> = {
    [BOT]: [thread(BOT, "main", "Main", null, 60 * 26), thread(BOT, PR_THREAD, "PR #120 · Add gift cards at checkout", "github:acme-labs/storefront#120", 18)],
  };
  return { bots, botChanges, activity, botQueued, botMemory, botThreads };
}
