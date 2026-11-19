// Demo routes: bots (Bots page). Creating a bot, editing its files and messaging it change the demo world; a
// message to a bot without an open session launches one that replays a prerecorded scenario.

import type { Bot } from "@/gen/Bot";
import type { BotSend } from "@/gen/BotSend";
import type { Session } from "@/gen/Session";
import type { BotThread } from "@/gen/BotThread";
import type { BotSettings } from "@/gen/BotSettings";
import type { LaunchSpec } from "@/gen/LaunchSpec";
import type { MessageContext } from "@/gen/MessageContext";
import { clock, DemoError, publish, route } from "../daemon";
import { world } from "../world";
import { ASK_DEFAULT, ASK_STARTERS, LIBRARIAN_DEFAULT, LIBRARIAN_STARTERS, settingsText } from "../world/bots";
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
const threadOf = (s: { launch_spec: LaunchSpec }) => s.launch_spec.bot?.thread || "main";
/** The sessions of one thread of a bot, oldest first (one per thread). */
const thread = (id: string, t: string) => world.sessions.filter((s) => s.launch_spec.bot?.id === id && threadOf(s) === t).sort((a, b) => a.started_at - b.started_at);
/** A bot's thread rows (Home's `bot_thread`). */
const threadsOf = (id: string): BotThread[] => (world.botThreads[id] ??= []);
const newThread = (b: Bot, key: string | null, title: string) => {
  const t: BotThread = { bot_id: b.id, id: `demo-thread-${Math.random().toString(36).slice(2, 10)}`, key, title: title.replace(/\s+/g, " ").trim().slice(0, 80) || "New thread", created_at: clock.now(), archived_at: null, conversation: "" };
  t.conversation = `${b.id}:${t.id}`;
  threadsOf(b.id).push(t);
  threadsChanged(b.id);
  return t.id;
};
const threadsChanged = (id: string) => publish("bots", "threads", { id });
/** The thread a message goes to (Home's `bot_threads::route` and `resolve`). */
function resolveThread(b: Bot, body: { thread?: string | null; new_thread?: boolean; item_thread?: boolean; conversation?: string | null; context?: MessageContext }, text: string): string {
  const list = threadsOf(b.id);
  const create = (key: string | null, title: string) => newThread(b, key, title);
  // A bot of a conversation: its open thread of it, else a new one that joins it.
  if (body.conversation) {
    const c = body.conversation;
    const open = list.find((t) => t.conversation === c && t.archived_at == null);
    if (open) return open.id;
    const main = Object.values(world.botThreads).flat().find((t) => t.conversation === c && t.bot_id === c.split(":")[0]);
    const id = create(`conversation:${c}`, main?.title ?? text.split("\n")[0] ?? "");
    list.find((t) => t.id === id)!.conversation = c;
    return id;
  }
  if (body.new_thread || !body.thread && !body.item_thread) return create(null, text.split("\n")[0] ?? "");
  if (body.item_thread) {
    const items = (body.context?.github_refs ?? []).filter((r) => r.number != null);
    const keys = [...new Set(items.map((r) => `github:${r.repo.toLowerCase()}#${r.number}`))];
    if (keys.length !== 1) return create(null, text.split("\n")[0] ?? "");
    const open = list.find((t) => t.key === keys[0] && t.archived_at == null);
    const r = items[0];
    return open?.id ?? create(keys[0], r.title ? `${r.kind === "pr" ? "PR " : r.kind === "issue" ? "Issue " : ""}#${r.number} · ${r.title}` : `${r.repo}#${r.number}`);
  }
  const t = list.find((x) => x.id === body.thread);
  if (!t) throw new DemoError(400, "this thread no longer exists");
  t.archived_at = null;
  return t.id;
}

/** The profile a bot runs with: its own, else the first subscription profile (Home's `bots::profile_of`). */
const profileOf = (b: Bot) => b.settings.profile ? world.config.profiles.find((p) => p.id === b.settings.profile) : world.config.profiles.find((p) => p.billing === "subscription") ?? world.config.profiles[0];
/** A bot's sessions start in its folder, or in a temporary folder without one. */
export const botTarget = (b: Bot): LaunchSpec["target"] => b.settings.folder ? { kind: "project", project_id: b.settings.folder } : { kind: "temp" };

/** A bot with its memory/ files listed (Home's `Bot.memory`). */
const withMemory = (b: Bot): Bot => ({ ...b, instructions_version: botVersion(b.instructions), memory_loaded: (({ loaded, total }) => ({ loaded, total }))(memoryParts(world.botMemory[b.id] ?? {})), routines: Object.fromEntries(world.crons.filter((c) => c.bot_id === b.id && c.file).map((c) => [c.file!, routineToml(defOf(c))])), memory: Object.entries(world.botMemory[b.id] ?? {}).map(([path, content]) => ({ path, sha256: `${b.id}-${path}-${content.length}`, size: content.length, executable: false })).sort((x, y) => x.path.localeCompare(y.path)) });

route("GET", "/api/bots", () => world.bots.map(withMemory));
route("GET", "/api/bots/templates", () => [{ id: "ask-launchpad", name: "Ask Launchpad", instructions: ASK_DEFAULT, starters: ASK_STARTERS }, { id: "librarian", name: "Librarian", instructions: LIBRARIAN_DEFAULT, starters: LIBRARIAN_STARTERS }]);
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
  const b: Bot = { id: `demo-bot-${slug}-${world.bots.length}`, workspace_id: c.workspace_id, slug, created_at: clock.now(), settings, instructions: c.instructions?.trim() ? `${c.instructions.trim()}\n` : `# ${settings.name}\n`, settings_text: settingsText(settings), settings_error: null, own_folder: false, paused: false, version: 1, memory: [], routines: {}, instructions_version: "", memory_loaded: { loaded: 0, total: 0 }, folder_unused: null };
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
  const r = body as { title?: string; instructions?: string; trigger?: CronAgent["trigger"]; schedule?: string; conditions?: CronAgent["conditions"]; time_zone?: string; auto_archive?: boolean; gate?: CronAgent["gate"] };
  if (!r.title?.trim()) throw new DemoError(400, "give the routine a name");
  if (!r.instructions?.trim()) throw new DemoError(400, "write what the bot does at each run");
  const profile = profileOf(b);
  // Its definition is a new file of the bot, routines/<name>.toml.
  const file = routinePath(r.title, world.crons.filter((c) => c.bot_id === b.id && c.file).map((c) => c.file!));
  const c = createCron({ ...defaultSpec(), harness: profile?.harness ?? "claude", profile: profile?.id ?? null, target: botTarget(b), text: r.instructions.trim(), workspace_id: b.workspace_id }, r.schedule ?? "", r.conditions ?? [], r.trigger ?? { kind: "schedule" }, { title: r.title.trim(), bot_id: b.id, bot_paused: b.paused, file, auto_archive: !!r.auto_archive, ...(r.time_zone ? { time_zone: r.time_zone } : {}), ...(r.gate ? { gate: r.gate } : {}) });
  record(b, file, "", routineToml(defOf(c)));
  return c;
});
route("DELETE", "/api/bots/:id", ({ params }) => {
  world.bots = world.bots.filter((b) => b.id !== params.id);
  world.crons = world.crons.filter((c) => c.bot_id !== params.id);
  publish("bots", "changed", { id: params.id });
  return { ok: true };
});
route("GET", "/api/bots/:id/threads", ({ params, query }) => { botOf(params.id); const all = query.get("archived") === "true"; return [...threadsOf(params.id)].map((t) => { const s = thread(params.id, t.id).at(-1); return t.archived_at == null && s?.status === "closed" ? { ...t, archived_at: s.status_since } : t; }).filter((t) => all || t.archived_at == null).sort((a, b) => b.created_at - a.created_at); });
route("PATCH", "/api/bots/:id/threads/:thread", ({ params, body }) => {
  const t = threadsOf(params.id).find((x) => x.id === params.thread);
  if (!t) throw new DemoError(400, "this thread no longer exists");
  const title = String((body as { title?: string })?.title ?? "").replace(/\s+/g, " ").trim().slice(0, 80);
  if (!title) throw new DemoError(400, "give the thread a title");
  t.title = title;
  threadsChanged(params.id);
  return t;
});
route("POST", "/api/bots/:id/threads/:thread/archive", ({ params, body }) => {
  const t = threadsOf(params.id).find((x) => x.id === params.thread);
  if (!t) throw new DemoError(400, "this thread no longer exists");
  const archived = (body as { archived?: boolean })?.archived !== false;
  if (archived) for (const s of thread(params.id, t.id).filter((x) => x.status !== "closed")) setStatus(s.id, "closed");
  else { const s = thread(params.id, t.id).at(-1); if (s?.status === "closed") setStatus(s.id, "waiting"); }
  t.archived_at = archived ? clock.now() : null;
  threadsChanged(params.id);
  return t;
});
route("GET", "/api/bots/:id/start-preview", ({ params, query }) => {
  const b = botOf(params.id);
  const profile = profileOf(b);
  const t = query.get("thread");
  const previous = t ? thread(b.id, t).at(-1) : undefined;
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
// The owner's message is stored, then sent (Home: `launchpad_channels::sends`); the demo sends it at once, so the
// list of accepted messages stays empty and `accept` answers with a receipt that is already sent.
route("GET", "/api/bots/:id/sends", () => []);
route("POST", "/api/bots/:id/message", (r) => {
  const s = sendBotMessage(r as never);
  const body = r.body as { accept?: boolean; text?: string } | undefined;
  if (!body?.accept) return s;
  const thread = s.launch_spec.bot?.thread ?? "";
  return { id: s.id, bot_id: r.params.id, thread, asked_thread: thread, text: String(body.text ?? ""), attachments: [], state: "sent", error: null, session_id: s.id, created_at: clock.now(), updated_at: clock.now() } satisfies BotSend;
});
function sendBotMessage({ params, body }: { params: Record<string, string>; body: unknown }): Session {
  const b = botOf(params.id);
  const text = withContext(String((body as { text?: string })?.text ?? ""), (body as { context?: MessageContext })?.context).trim();
  if (!text) throw new DemoError(400, "the message is empty");
  let threadId = resolveThread(b, body as never, text);
  const current = thread(b.id, threadId).at(-1);
  const profile = profileOf(b);
  // Home's `bots::rotation`: archived, or a new profile, model or permission mode, continues in a new thread.
  const reason = !current ? null
    : current.status === "closed" ? "archived"
    : current.launch_spec.profile !== (profile?.id ?? null) ? "profile"
    : (current.launch_spec.model ?? null) !== (b.settings.model ?? profile?.model ?? null) ? "model"
    : (current.launch_spec.bot?.permission_mode ?? current.launch_spec.permission_mode) !== b.settings.permission_mode ? "permissions" : null;
  if (current && !reason) {
    // BOT.md changed since the session read it: the new one goes after the typed text (`bots::with_new_instructions`).
    const v = botVersion(b.instructions);
    const bot = current.launch_spec.bot;
    const changedText = bot?.version && bot.version !== v ? `${text}\n\nYour instructions changed. The owner edited BOT.md since this session started or last heard of it. From now on, follow this version instead of the earlier one:\n\n<bot-instructions>\n${b.instructions.trim()}\n</bot-instructions>` : text;
    if (bot && bot.version !== v) { bot.version = v; publishSession(current); }
    // A message instead of an answer (Cancel on “Answering: …”): the bot reads it and drops its question.
    if (current.open_question) { current.open_question = false; publishSession(current); }
    sendMessage(current, changedText, `${current.id}-${current.last_seq + 1}`);
    return session(current.id)!;
  }
  // `bot_threads::follow_up`: a new thread with the same key and title; the old one leaves the bar.
  if (current) {
    if (current.status !== "closed") setStatus(current.id, "closed");
    const old = threadsOf(b.id).find((t) => t.id === threadId);
    threadId = newThread(b, old?.key ?? null, old?.title ?? "");
    if (old) old.archived_at = clock.now();
  }
  const previous = current;
  const spec = { harness: profile?.harness ?? "claude", profile: profile?.id ?? null, target: botTarget(b), worktree: !b.settings.folder || world.projects.find((p) => p.id === b.settings.folder)?.plain ? null : { base: null, when_done: "keep" }, workspace_id: b.workspace_id, text,
    model: b.settings.model ?? profile?.model ?? null, permission_mode: b.settings.permission_mode,
    bot: { id: b.id, name: b.settings.name, slug: b.slug, instructions: "", thread: threadId, version: botVersion(b.instructions), reason: reason ?? "first", permission_mode: b.settings.permission_mode, previous: previous ? { session_id: previous.id, title: previous.title, through_seq: previous.last_seq } : null } } as Partial<LaunchSpec> as LaunchSpec;
  return launch(spec, undefined, b.slug === "ask-launchpad" ? pickReply(text, true) : undefined);
}
