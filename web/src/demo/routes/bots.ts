// Demo routes: bots (Bots page). Creating a bot, editing its files and messaging it change the demo world; a
// message to a bot without an open session launches one that replays a prerecorded scenario.

import type { Bot } from "@/gen/Bot";
import type { BotSettings } from "@/gen/BotSettings";
import type { LaunchSpec } from "@/gen/LaunchSpec";
import type { MessageContext } from "@/gen/MessageContext";
import { clock, DemoError, publish, route } from "../daemon";
import { world } from "../world";
import { ASK_DEFAULT, ASK_STARTERS, settingsText } from "../world/bots";
import { publishSession, session, setStatus } from "../store";
import { applyRoutineFile, createCron, launch, memoryParts, nextDue, recordBotChange, routinePath, startPreview } from "./launch";
import { botVersion, defOf, routineToml } from "../routine";
import type { CronAgent } from "@/gen/CronAgent";
import { defaultSpec } from "../store";
import { sendMessage, withContext } from "./agents";
import { pickReply } from "../scenarios";

const botOf = (id: string) => {
  const b = world.bots.find((x) => x.id === id);
  if (!b) throw new DemoError(404, "no such bot");
  return b;
};
const changed = (b: Bot) => { b.version += 1; publish("bots", "changed", { id: b.id }); };
const record = (b: Bot, path: string, before: string, after: string) => recordBotChange(b.id, path, before || null, after);
const thread = (id: string) => world.sessions.filter((s) => s.launch_spec.bot?.id === id).sort((a, b) => a.started_at - b.started_at);

/** The profile a bot runs with: its own, else the first subscription profile (Home's `bots::profile_of`). */
const profileOf = (b: Bot) => b.settings.profile ? world.config.profiles.find((p) => p.id === b.settings.profile) : world.config.profiles.find((p) => p.billing === "subscription") ?? world.config.profiles[0];
/** A bot's sessions start in its folder, or in a temporary folder without one. */
export const botTarget = (b: Bot): LaunchSpec["target"] => b.settings.folder ? { kind: "project", project_id: b.settings.folder } : { kind: "temp" };

/** A bot with its memory/ files listed (Home's `Bot.memory`). */
const withMemory = (b: Bot): Bot => ({ ...b, instructions_version: botVersion(b.instructions), memory_loaded: (({ loaded, total }) => ({ loaded, total }))(memoryParts(world.botMemory[b.id] ?? {})), routines: Object.fromEntries(world.crons.filter((c) => c.bot_id === b.id && c.file).map((c) => [c.file!, routineToml(defOf(c))])), memory: Object.entries(world.botMemory[b.id] ?? {}).map(([path, content]) => ({ path, sha256: `${b.id}-${path}-${content.length}`, size: content.length, executable: false })).sort((x, y) => x.path.localeCompare(y.path)) });

route("GET", "/api/bots", () => world.bots.map(withMemory));
route("GET", "/api/bots/templates", () => [{ id: "ask-launchpad", name: "Ask Launchpad", instructions: ASK_DEFAULT, starters: ASK_STARTERS }]);
route("GET", "/api/bots/:id", ({ params }) => withMemory(botOf(params.id)));
route("GET", "/api/bots/:id/file", ({ params, query }) => {
  const b = botOf(params.id);
  const path = query.get("path") ?? "";
  const routine = world.crons.find((c) => c.bot_id === b.id && c.file === path);
  const text = path === "BOT.md" ? b.instructions : path === "bot.toml" ? b.settings_text : routine ? routineToml(defOf(routine)) : world.botMemory[b.id]?.[path];
  if (text == null) throw new DemoError(404, `there is no ${path}`);
  return new Response(text, { headers: { "Content-Type": "text/plain; charset=utf-8" } });
});
route("POST", "/api/bots", ({ body }) => {
  const c = body as { workspace_id: string; name: string; instructions: string; profile: string | null; folder: string | null; starters?: string[] };
  if (!c.name?.trim()) throw new DemoError(400, "give the bot a name");
  const slug = c.name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || "bot";
  // No folder: each session starts in a temporary folder.
  const settings: BotSettings = { name: c.name.trim(), profile: c.profile ?? undefined, folder: c.folder || undefined, permission_mode: "yolo", proposals: "manual", launchpad: "propose", ...(c.starters?.length ? { starters: c.starters } : {}) };
  const b: Bot = { id: `demo-bot-${slug}-${world.bots.length}`, workspace_id: c.workspace_id, slug, created_at: clock.now(), settings, instructions: c.instructions?.trim() ? `${c.instructions.trim()}\n` : `# ${settings.name}\n`, settings_text: settingsText(settings), settings_error: null, own_folder: false, paused: false, version: 1, memory: [], routines: {}, instructions_version: "", memory_loaded: { loaded: 0, total: 0 } };
  world.bots.push(b);
  record(b, "BOT.md", "", b.instructions);
  changed(b);
  return b;
});
route("PATCH", "/api/bots/:id", ({ params, body }) => {
  const b = botOf(params.id);
  const p = body as { settings?: BotSettings; instructions?: string; settings_text?: string; set?: Record<string, unknown>; files?: Record<string, string | null> };
  if (p.set) p.settings = Object.fromEntries(Object.entries({ ...b.settings, ...p.set }).filter(([, v]) => v != null)) as BotSettings;
  if (p.settings_text != null) throw new DemoError(400, "Demo: edit the settings with the form in About → Access.");
  if (p.settings && p.settings.name.trim() !== b.settings.name) {
    // A new name gives a new slug, unique in the workspace (Home's `bots::rename`).
    const base = p.settings.name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || "bot";
    const taken = world.bots.filter((x) => x.workspace_id === b.workspace_id && x.id !== b.id).map((x) => x.slug);
    let slug = base;
    for (let n = 2; taken.includes(slug); n++) slug = `${base}-${n}`;
    b.slug = slug;
  }
  if (p.settings) { const before = b.settings_text; b.settings = { ...p.settings }; b.settings_text = settingsText(b.settings); record(b, "bot.toml", before, b.settings_text); }
  if (p.instructions != null) { const before = b.instructions; b.instructions = p.instructions; record(b, "BOT.md", before, b.instructions); }
  for (const [path, text] of Object.entries(p.files ?? {})) applyRoutineFile(b.id, path, text);
  changed(b);
  return withMemory(b);
});
route("POST", "/api/bots/:id/pause", ({ params, body }) => {
  const b = botOf(params.id);
  b.paused = !!(body as { paused?: boolean })?.paused;
  for (const c of world.crons.filter((x) => x.bot_id === b.id)) { c.bot_paused = b.paused; c.next_run_at = nextDue(c); }
  publish("crons", "changed", null);
  changed(b);
  return withMemory(b);
});
route("POST", "/api/bots/:id/routines", ({ params, body }) => {
  const b = botOf(params.id);
  const r = body as { title?: string; instructions?: string; trigger?: CronAgent["trigger"]; schedule?: string; conditions?: CronAgent["conditions"]; time_zone?: string };
  if (!r.title?.trim()) throw new DemoError(400, "give the routine a name");
  if (!r.instructions?.trim()) throw new DemoError(400, "write what the bot does at each run");
  const profile = profileOf(b);
  // Its definition is a new file of the bot, routines/<name>.toml.
  const file = routinePath(r.title, world.crons.filter((c) => c.bot_id === b.id && c.file).map((c) => c.file!));
  const c = createCron({ ...defaultSpec(), harness: profile?.harness ?? "claude", profile: profile?.id ?? null, target: botTarget(b), text: r.instructions.trim(), workspace_id: b.workspace_id }, r.schedule ?? "", r.conditions ?? [], r.trigger ?? { kind: "schedule" }, { title: r.title.trim(), bot_id: b.id, bot_paused: b.paused, file, ...(r.time_zone ? { time_zone: r.time_zone } : {}) });
  record(b, file, "", routineToml(defOf(c)));
  return c;
});
route("DELETE", "/api/bots/:id", ({ params }) => {
  world.bots = world.bots.filter((b) => b.id !== params.id);
  world.crons = world.crons.filter((c) => c.bot_id !== params.id);
  publish("bots", "changed", { id: params.id });
  return { ok: true };
});
route("GET", "/api/bots/:id/start-preview", ({ params }) => {
  const b = botOf(params.id);
  const profile = profileOf(b);
  const previous = thread(b.id).at(-1);
  return startPreview({ ...defaultSpec(), harness: profile?.harness ?? "claude", profile: profile?.id ?? null, target: botTarget(b), workspace_id: b.workspace_id, text: "(your next message)",
    bot: { id: b.id, name: b.settings.name, slug: b.slug, instructions: b.instructions, previous: previous ? { session_id: previous.id, title: previous.title, through_seq: previous.last_seq } : null } });
});
route("GET", "/api/bots/:id/queued", ({ params }) => world.botQueued[params.id] ?? []);
route("DELETE", "/api/bots/:id/queued/:n", ({ params }) => {
  const list = world.botQueued[params.id] ?? [];
  if (!list.some((m) => m.n === Number(params.n))) throw new DemoError(400, "this message is no longer waiting: it was sent or cancelled");
  world.botQueued[params.id] = list.filter((m) => m.n !== Number(params.n));
  publish("bots", "queued", { id: params.id });
  return { ok: true };
});
route("GET", "/api/bots/:id/history", ({ params }) => (world.botChanges[params.id] ?? []).map(({ patch: _patch, ...c }) => c));
route("GET", "/api/bots/:id/history/:change", ({ params }) => ({ patch: (world.botChanges[params.id] ?? []).find((c) => c.id === Number(params.change))?.patch ?? "" }));
route("POST", "/api/bots/:id/restore/:change", () => { throw new DemoError(400, "Demo: restoring a version works on a real Home."); });
route("POST", "/api/bots/:id/message", ({ params, body }) => {
  const b = botOf(params.id);
  const text = withContext(String((body as { text?: string })?.text ?? ""), (body as { context?: MessageContext })?.context).trim();
  if (!text) throw new DemoError(400, "the message is empty");
  const sessions = thread(b.id);
  const current = [...sessions].reverse().find((s) => s.status !== "closed");
  const profile = profileOf(b);
  // Home's `bots::rotation`: a new profile, model or permission mode starts a new session.
  const reason = !current ? (sessions.length ? "archived" : "first")
    : current.launch_spec.profile !== (profile?.id ?? null) ? "profile"
    : (current.launch_spec.model ?? null) !== (b.settings.model ?? profile?.model ?? null) ? "model"
    : (current.launch_spec.bot?.permission_mode ?? current.launch_spec.permission_mode) !== b.settings.permission_mode ? "permissions" : null;
  const fresh = !!(body as { new_session?: boolean }).new_session;
  if (current && !fresh && !reason) {
    // BOT.md changed since the session read it: the new one goes after the typed text (`bots::with_new_instructions`).
    const v = botVersion(b.instructions);
    const bot = current.launch_spec.bot;
    const changedText = bot?.version && bot.version !== v ? `${text}\n\nYour instructions changed. The owner edited BOT.md since this session started or last heard of it. From now on, follow this version instead of the earlier one:\n\n<bot-instructions>\n${b.instructions.trim()}\n</bot-instructions>` : text;
    if (bot && bot.version !== v) { bot.version = v; publishSession(current); }
    // A message instead of an answer (Cancel on “Answering: …”): the bot reads it and drops its question.
    if (current.open_question) { current.open_question = false; publishSession(current); }
    sendMessage(current, changedText, `${current.id}-${current.last_seq + 1}`);
    return session(current.id);
  }
  if (current) setStatus(current.id, "closed");
  const previous = sessions.at(-1);
  const spec = { harness: profile?.harness ?? "claude", profile: profile?.id ?? null, target: botTarget(b), worktree: !b.settings.folder || world.projects.find((p) => p.id === b.settings.folder)?.plain ? null : { base: null, when_done: "keep" }, workspace_id: b.workspace_id, text,
    model: b.settings.model ?? profile?.model ?? null, permission_mode: b.settings.permission_mode,
    bot: { id: b.id, name: b.settings.name, slug: b.slug, instructions: "", version: botVersion(b.instructions), reason: fresh && !reason ? "new" : reason ?? "first", note: fresh ? String((body as { reason?: string }).reason ?? "").trim().slice(0, 120) || undefined : undefined, permission_mode: b.settings.permission_mode, previous: previous ? { session_id: previous.id, title: previous.title, through_seq: previous.last_seq } : null } } as Partial<LaunchSpec> as LaunchSpec;
  return launch(spec, undefined, b.slug === "ask-launchpad" ? pickReply(text, true) : undefined);
});
