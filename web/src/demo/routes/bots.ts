// Demo routes: bots (Bots page). Creating a bot, editing its files and messaging it change the demo world; a
// message to a bot without an open session launches one that replays a prerecorded scenario.

import type { Bot } from "@/gen/Bot";
import type { BotSettings } from "@/gen/BotSettings";
import type { LaunchSpec } from "@/gen/LaunchSpec";
import { clock, DemoError, publish, route } from "../daemon";
import { world } from "../world";
import { ASK_DEFAULT, settingsText } from "../world/bots";
import { session, setStatus } from "../store";
import { createCron, launch, nextDue } from "./launch";
import type { CronAgent } from "@/gen/CronAgent";
import { defaultSpec } from "../store";
import { sendMessage } from "./agents";
import { pickReply } from "../scenarios";

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

/** The profile a bot runs with: its own, else the first subscription profile (Home's `bots::profile_of`). */
const profileOf = (b: Bot) => b.settings.profile ? world.config.profiles.find((p) => p.id === b.settings.profile) : world.config.profiles.find((p) => p.billing === "subscription") ?? world.config.profiles[0];

route("GET", "/api/bots", () => world.bots);
route("GET", "/api/bots/templates", () => [{ id: "ask-launchpad", name: "Ask Launchpad", instructions: ASK_DEFAULT }]);
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
  const settings: BotSettings = { name: c.name.trim(), profile: c.profile ?? undefined, folder, permission_mode: "yolo", proposals: "manual", launchpad: "propose" };
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
  const profile = profileOf(b);
  return createCron({ ...defaultSpec(), harness: profile?.harness ?? "claude", profile: profile?.id ?? null, target: { kind: "project", project_id: b.settings.folder }, text: r.instructions.trim(), workspace_id: b.workspace_id }, r.schedule ?? "", r.conditions ?? [], r.trigger ?? { kind: "schedule" }, { title: r.title.trim(), bot_id: b.id, bot_paused: b.paused, ...(r.time_zone ? { time_zone: r.time_zone } : {}) });
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
  const profile = profileOf(b);
  if (current && !(body as { new_session?: boolean }).new_session && current.launch_spec.profile === (profile?.id ?? null)) {
    sendMessage(current, text, `${current.id}-${current.last_seq + 1}`);
    return session(current.id);
  }
  if (current) setStatus(current.id, "closed");
  const previous = sessions.at(-1);
  const spec = { harness: profile?.harness ?? "claude", profile: profile?.id ?? null, target: { kind: "project", project_id: b.settings.folder }, worktree: world.projects.find((p) => p.id === b.settings.folder)?.plain ? null : { base: null, when_done: "keep" }, workspace_id: b.workspace_id, text,
    bot: { id: b.id, name: b.settings.name, slug: b.slug, instructions: "", previous: previous ? { session_id: previous.id, title: previous.title, through_seq: previous.last_seq } : null } } as Partial<LaunchSpec> as LaunchSpec;
  return launch(spec, undefined, b.slug === "ask-launchpad" ? pickReply(text, true) : undefined);
});
