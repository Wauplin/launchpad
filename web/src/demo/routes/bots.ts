// Demo routes: bots (Bots page). Creating a bot, editing its files and messaging it change the demo world; a
// message to a bot without an open session launches one that replays a prerecorded scenario.

import type { Bot } from "@/gen/Bot";
import type { BotSettings } from "@/gen/BotSettings";
import type { LaunchSpec } from "@/gen/LaunchSpec";
import { clock, DemoError, publish, route } from "../daemon";
import { world } from "../world";
import { ASK_DEFAULT, settingsText } from "../world/bots";
import type { AskBot } from "@/gen/AskBot";
import { session, setStatus } from "../store";
import { createCron, launch, nextDue } from "./launch";
import type { CronAgent } from "@/gen/CronAgent";
import { defaultSpec } from "../store";
import { sendMessage } from "./agents";

const botOf = (id: string) => {
  const b = world.bots.find((x) => x.id === id);
  if (!b) throw new DemoError(404, "no such bot");
  return b;
};
const changed = (b: Bot) => { b.version += 1; publish("bots", "changed", { id: b.id }); };
const record = (b: Bot, path: string, before: string, after: string) => {
  const list = (world.botChanges[b.id] ??= []);
  const id = (list[0]?.id ?? 0) + 1;
  const patch = `@@ -1 +1 @@\n${before.trimEnd().split("\n").map((l) => `-${l}`).join("\n")}\n${after.trimEnd().split("\n").map((l) => `+${l}`).join("\n")}\n`;
  list.unshift({ id, path, sha256: before ? `old-${id}` : null, new_sha256: `new-${id}`, at: clock.now(), session_id: null, author: { kind: "owner", name: "You", session_id: null, model: null, runner: null }, added: after.split("\n").length, removed: before ? before.split("\n").length : 0, patch });
};
const thread = (id: string) => world.sessions.filter((s) => s.launch_spec.bot?.id === id).sort((a, b) => a.started_at - b.started_at);

route("GET", "/api/bots", () => world.bots);
route("GET", "/api/bots/:id", ({ params }) => botOf(params.id));
route("POST", "/api/bots", ({ body }) => {
  const c = body as { workspace_id: string; name: string; instructions: string; profile: string | null; folder: string | null };
  if (!c.name?.trim()) throw new DemoError(400, "give the bot a name");
  const slug = c.name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || "bot";
  let folder = c.folder;
  if (!folder) {
    const id = `demo-bot-folder-${slug}`;
    world.projects.push({ ...world.projects[0], id, name: c.name.trim(), path: `/home/demo/.local/share/launchpad/plain/${id}`, gh_repo: null, gh_upstream: null, remote_url: null, description: "", is_temp: false, pinned: false, workspace_id: c.workspace_id, plain: true, home_git: false, created_at: clock.now() });
    publish("projects", "changed", null);
    folder = id;
  }
  const settings: BotSettings = { name: c.name.trim(), profile: c.profile ?? undefined, folder, permission_mode: "yolo" };
  const b: Bot = { id: `demo-bot-${slug}-${world.bots.length}`, workspace_id: c.workspace_id, slug, created_at: clock.now(), settings, instructions: c.instructions?.trim() ? `${c.instructions.trim()}\n` : `# ${settings.name}\n`, settings_text: settingsText(settings), settings_error: null, own_folder: !c.folder, paused: false, version: 1 };
  world.bots.push(b);
  record(b, "BOT.md", "", b.instructions);
  changed(b);
  return b;
});
route("PATCH", "/api/bots/:id", ({ params, body }) => {
  const b = botOf(params.id);
  const p = body as { settings?: BotSettings; instructions?: string; settings_text?: string };
  if (p.settings_text != null) throw new DemoError(400, "Demo: edit the settings with the form in the Settings tab.");
  if (p.settings) { const before = b.settings_text; b.settings = { ...p.settings }; b.settings_text = settingsText(b.settings); record(b, "bot.toml", before, b.settings_text); }
  if (p.instructions != null) { const before = b.instructions; b.instructions = p.instructions; record(b, "BOT.md", before, b.instructions); }
  changed(b);
  return b;
});
route("POST", "/api/bots/:id/pause", ({ params, body }) => {
  const b = botOf(params.id);
  b.paused = !!(body as { paused?: boolean })?.paused;
  for (const c of world.crons.filter((x) => x.bot_id === b.id)) { c.bot_paused = b.paused; c.next_run_at = nextDue(c); }
  publish("crons", "changed", null);
  changed(b);
  return b;
});
route("POST", "/api/bots/:id/routines", ({ params, body }) => {
  const b = botOf(params.id);
  const r = body as { title?: string; instructions?: string; trigger?: CronAgent["trigger"]; schedule?: string; conditions?: CronAgent["conditions"]; time_zone?: string };
  if (!r.title?.trim()) throw new DemoError(400, "give the routine a name");
  if (!r.instructions?.trim()) throw new DemoError(400, "write what the bot does at each run");
  const profile = world.config.profiles.find((p) => p.id === b.settings.profile);
  return createCron({ ...defaultSpec(), harness: profile?.harness ?? "claude", profile: b.settings.profile ?? null, target: { kind: "project", project_id: b.settings.folder }, text: r.instructions.trim(), workspace_id: b.workspace_id }, r.schedule ?? "", r.conditions ?? [], r.trigger ?? { kind: "schedule" }, { title: r.title.trim(), bot_id: b.id, bot_paused: b.paused, ...(r.time_zone ? { time_zone: r.time_zone } : {}) });
});
route("DELETE", "/api/bots/:id", ({ params }) => {
  world.bots = world.bots.filter((b) => b.id !== params.id);
  world.crons = world.crons.filter((c) => c.bot_id !== params.id);
  publish("bots", "changed", { id: params.id });
  return { ok: true };
});
route("GET", "/api/bots/:id/history", ({ params }) => (world.botChanges[params.id] ?? []).map(({ patch: _patch, ...c }) => c));
route("GET", "/api/bots/:id/history/:change", ({ params }) => ({ patch: (world.botChanges[params.id] ?? []).find((c) => c.id === Number(params.change))?.patch ?? "" }));
route("POST", "/api/bots/:id/restore/:change", () => { throw new DemoError(400, "Demo: restoring a version works on a real Home."); });
route("POST", "/api/bots/:id/message", ({ params, body }) => {
  const b = botOf(params.id);
  const text = String((body as { text?: string })?.text ?? "").trim();
  if (!text) throw new DemoError(400, "the message is empty");
  const sessions = thread(b.id);
  const current = [...sessions].reverse().find((s) => s.status !== "closed");
  if (current && !(body as { new_session?: boolean }).new_session && current.launch_spec.profile === (b.settings.profile ?? null)) {
    sendMessage(current, text, `${current.id}-${current.last_seq + 1}`);
    return session(current.id);
  }
  if (current) setStatus(current.id, "closed");
  const previous = sessions.at(-1);
  const profile = world.config.profiles.find((p) => p.id === b.settings.profile);
  const spec = { harness: profile?.harness ?? "claude", profile: b.settings.profile ?? null, target: { kind: "project", project_id: b.settings.folder }, worktree: world.projects.find((p) => p.id === b.settings.folder)?.plain ? null : { base: null, when_done: "keep" }, workspace_id: b.workspace_id, text,
    bot: { id: b.id, name: b.settings.name, slug: b.slug, instructions: "", previous: previous ? { session_id: previous.id, title: previous.title, through_seq: previous.last_seq } : null } } as Partial<LaunchSpec> as LaunchSpec;
  return launch(spec);
});

// Ask Launchpad, the built-in bot: BOT.md with versions and Reset to default.
const ASK = "ask-launchpad";
const askBot = (): AskBot => ({
  instructions: world.askBot.instructions, default_instructions: ASK_DEFAULT, version: world.askBot.version,
  builtin: "You are Ask Launchpad, the owner's assistant inside Launchpad, scoped to the current workspace. You turn requests into proposals. You never start agents, send messages, schedule, publish or post to GitHub yourself, and you do not edit files. These rules and the reference below are built into Launchpad. The owner's instructions (BOT.md) say how they want you to work within them; when the two conflict, these rules win.\n\n<bot-instructions>\n(your BOT.md)\n</bot-instructions>\n\nRead Launchpad state with these read-only commands (JSON output): launchpad folders list, bots list, sessions list, profiles list, presets list, inbox list, schedules list, proposals list, trace <session-id>.\n\nWrite proposals with launchpad propose <launch|message|schedule|cron|status|preset|tool|bot|routine|bot-instructions> …",
  customized: world.askBot.instructions !== ASK_DEFAULT || world.config.assistant.profile != null || world.config.assistant.model != null || world.config.assistant.auto_mode,
});
const setAsk = (text: string) => {
  const b = { id: ASK } as Bot;
  if (text !== world.askBot.instructions) record(b, "BOT.md", world.askBot.instructions, text);
  world.askBot.instructions = text;
  world.askBot.version += 1;
  publish("bots", "changed", { id: ASK });
  return askBot();
};
route("GET", "/api/assistant/bot", askBot);
route("PATCH", "/api/assistant/bot", ({ body }) => {
  const text = (body as { instructions: string }).instructions ?? "";
  if (!text.trim()) throw new DemoError(400, "BOT.md is empty: use Reset to go back to the default");
  return setAsk(text.endsWith("\n") ? text : `${text}\n`);
});
route("GET", "/api/assistant/bot/history", () => (world.botChanges[ASK] ?? []).map(({ patch: _patch, ...c }) => c));
route("GET", "/api/assistant/bot/history/:change", ({ params }) => ({ patch: (world.botChanges[ASK] ?? []).find((c) => c.id === Number(params.change))?.patch ?? "" }));
route("POST", "/api/assistant/bot/restore/:change", () => { throw new DemoError(400, "Demo: restoring a version works on a real Home."); });
route("POST", "/api/assistant/bot/reset", () => {
  world.config = { ...world.config, assistant: { profile: null, model: null, effort: null, auto_mode: false } };
  publish("config", "changed", null);
  return setAsk(ASK_DEFAULT);
});
