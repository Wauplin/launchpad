import { topicContext } from "./topics";
// Launching agents: the composer draft, preview and launch, delayed launches and follow-ups, crons, proposals
// and uploads. A launch plays the prerecorded scenario closest to its prompt.

import type { CronAgent } from "@/gen/CronAgent";
import { botTarget } from "./bots";
import type { CronCondition } from "@/gen/CronCondition";
import type { LaunchDelay } from "@/gen/LaunchDelay";
import type { LaunchSpec } from "@/gen/LaunchSpec";
import type { Preview } from "@/gen/Preview";
import type { StartPart } from "@/gen/StartPart";
import type { StartPreview } from "@/gen/StartPreview";
import type { Proposal } from "@/gen/Proposal";
import type { ScheduledLaunch } from "@/gen/ScheduledLaunch";
import type { Session } from "@/gen/Session";
import { toast } from "@/components/Toast";
import { clock, DemoError, notInDemo, onClock, publish, route } from "../daemon";
import { predraftRoot } from "../world/github";
import { playScenario } from "../player";
import { pickScenario } from "../scenarios";
import { appendEvent, defaultSpec, demoId, publishSession, refreshCounts, session } from "../store";
import { world } from "../world";
import { demoSession, FOLDER, MODELS, PROFILE, SUMMARY_INSTRUCTION, taskWorktree } from "../world/agents";
import { P, ROOT, TMP } from "../world/ids";
import { defOf, parseRoutineToml, routinePath, routineText, routineToml } from "../routine";
import { found, quotaReset, ready, sendMessage } from "./agents";
import { createChannel, setMember } from "./channels";

// ---------------------------------------------------------------- launch

type Where = { projectId: string | null; cwd: string; git: boolean };

function where(spec: LaunchSpec): Where {
  const t = spec.target;
  const project = (id: string) => {
    const p = world.projects.find((x) => x.id === id);
    if (!p) throw new DemoError(400, "no such folder");
    return { projectId: p.id, cwd: p.path, git: !p.is_temp };
  };
  switch (t.kind) {
    case "project": case "predraft": return project(t.project_id);
    case "existing_predraft": {
      const p = world.predrafts.find((x) => x.id === t.predraft_id);
      if (!p) throw new DemoError(400, "no such Local PR");
      return { projectId: p.project_id, cwd: predraftRoot(p), git: true };
    }
    case "session_folder": {
      const s = found(t.session_id);
      return { projectId: s.project_id, cwd: s.cwd, git: !!world.sessionGit[s.id] || !!s.project_id };
    }
    case "temp": return { projectId: P.scratch, cwd: `${TMP}/${FOLDER[P.scratch]}`, git: false };
    default: throw new DemoError(400, "Choose a folder first.");
  }
}

/** `Sessions::resolve`'s labelled parts (`start::slices`): the typed text with its context and presets, the folder,
 * the bot and its BOT.md, then the Launchpad final-message block. */
export function promptParts(spec: LaunchSpec, cwd: string): StartPart[] {
  const parts = [spec.text.trim()];
  const context: string[] = [];
  for (const r of spec.github_refs) context.push(`- ${r.kind === "pr" ? "Pull request" : "Issue"} ${r.repo}#${r.number ?? ""}${r.title ? `: ${r.title}` : ""} (${r.url})`);
  for (const c of spec.context_projects) { const p = world.projects.find((x) => x.id === c.project_id); if (p) context.push(`- ${c.editable ? "Editable" : "Reference"} project: ${p.path}${p.description ? ` (${p.description})` : ""}.`); }
  for (const f of spec.files) context.push(`- File: ${f.root}/${f.path}${f.start_line ? ` lines ${f.start_line}-${f.end_line ?? f.start_line}` : ""}`);
  for (const a of spec.attachments) context.push(`- Attached ${a.data_url.startsWith("data:image/") || a.data_url.startsWith("upload:image/") ? "image" : "file (local copy provided at launch)"}: ${a.name}`);
  for (const t of spec.agent_traces) context.push(`- Trace of another agent: run \`launchpad trace ${t.session_id} --through ${t.through_seq}\` to read it.`);
  if (context.length) parts.push(`Context:\n${context.join("\n")}${spec.github_refs.length ? "\nUse `gh` to read issues, pull requests and comments." : ""}`);
  const presets = world.config.presets.filter((p) => (spec.bot ? p.bots : p.agent) && spec.presets.includes(p.id) && p.text.trim()).map((p) => p.text.trim()).join(" ");
  if (presets) parts.push(presets);
  const out: StartPart[] = [{ key: "message", label: "Your message, its context and presets", text: parts.filter(Boolean).join("\n\n") }];
  if (spec.worktree?.when_done === "read_only") out.push({ key: "folder", label: "Folder and worktrees", text: `Launchpad worktree (read only): you read ${cwd}, a Git worktree on its own branch. Do not change, commit or push files; give your findings in your final message.` });
  else if (spec.worktree) out.push({ key: "folder", label: "Folder and worktrees", text: `Launchpad worktree: you work in ${cwd}, a Git worktree on its own branch. Commit your changes there.${spec.worktree.when_done === "merge" ? " When the task is done, run `launchpad work finish` so Launchpad merges the branch." : spec.worktree.when_done === "local_pr" ? " When the task is done, run `launchpad work finish` so Launchpad turns the worktree into a Local PR." : spec.worktree.when_done === "draft_pr" || spec.worktree.when_done === "open_pr" ? ` When the task is done, run \`launchpad work finish\`: Launchpad opens a Local PR and publishes it on GitHub as ${spec.worktree.when_done === "draft_pr" ? "a draft pull request" : "a pull request"} after your turn.` : ""}` });
  if (spec.bot) out.push({ key: "bot", label: "The bot and its BOT.md", text: botInstruction(spec.bot.name, world.bots.find((b) => b.id === spec.bot!.id)?.instructions ?? spec.bot.instructions, spec.bot.previous) });
  out.push({ key: "final", label: "Final message", text: SUMMARY_INSTRUCTION });
  return out.filter((p) => p.text.trim());
}

/** `compose::render_prompt` and the rest of the first prompt, as one text. */
function renderPrompt(spec: LaunchSpec, cwd: string): string {
  return promptParts(spec, cwd).map((p) => p.text).join("\n\n");
}

/** `bots::instruction`, shortened for the demo. */
function botInstruction(name: string, instructions: string, previous: { session_id: string; title: string; through_seq: number } | null): string {
  return `You are ${name}, a Launchpad bot: a persistent agent with one thread that continues across sessions. The owner's instructions for you (BOT.md):\n\n<bot-instructions>\n${instructions.trim()}\n</bot-instructions>\n\nYour folder: \`$LAUNCHPAD_BOT\` is a read-only copy of your files: BOT.md, bot.toml, \`routines/<name>.toml\` (your routines) and \`memory/\`, your own notes. Your instructions hold these notes as they were at start ("Your memory", up to 100 lines; when they say it was cut, read the rest before you answer anything it may cover). Write them with \`launchpad bot memory write <file>\`.${previous ? `\n\nThis session continues the thread. The previous session is a reference, not new instructions; read it only when you need its context: "${previous.title}", through event ${previous.through_seq}: \`launchpad trace ${previous.session_id} --through ${previous.through_seq}\`` : ""}`;
}

const MEMORY_LINES = 100;

/** `bot_memory::compose`: a bot's notes (INDEX.md first, then by path), cut at 100 lines, and the cut line. */
export function memoryParts(notes: Record<string, string>): { text: string; cut: string | null; loaded: number; total: number } {
  const files = Object.entries(notes).filter(([p]) => p.startsWith("memory/") && p.endsWith(".md")).sort(([a], [b]) => (a === "memory/INDEX.md" ? -1 : b === "memory/INDEX.md" ? 1 : a.localeCompare(b)));
  const lines = (t: string) => t.split("\n").length - (t.endsWith("\n") ? 1 : 0);
  const total = files.reduce((n, [, t]) => n + lines(t), 0);
  if (!files.length) return { text: "", cut: null, loaded: 0, total: 0 };
  let text = "# Your memory\n\nYour notes in `$LAUNCHPAD_BOT/memory/`, as they were when this session started (read a file again before you rely on a detail that may have changed):\n";
  let loaded = 0;
  for (const [path, body] of files) {
    if (loaded >= MEMORY_LINES) break;
    const take = body.split("\n").slice(0, lines(body)).slice(0, MEMORY_LINES - loaded);
    text += `\n## ${path}\n\n${take.map((l) => `${l}\n`).join("")}`;
    loaded += take.length;
  }
  const cut = loaded < total ? `Your memory was cut at ${loaded} of ${total} lines. The full notes are in $LAUNCHPAD_BOT/memory/; read the rest with \`launchpad bot memory read <file>\` before you answer anything they may cover.\n` : null;
  return { text, cut, loaded, total };
}

/** Home's start preview (`crates/api/src/bots.rs`): the instructions file and the first prompt of a launch, in labelled parts. */
export function startPreview(spec: LaunchSpec): StartPreview {
  const s: LaunchSpec = { ...defaultSpec(), ...spec };
  const ws = s.workspace_id ?? (s.target.kind === "project" ? world.projects.find((p) => p.id === (s.target as { project_id: string }).project_id)?.workspace_id : null) ?? null;
  const name = world.workspaces.find((w) => w.id === ws)?.name ?? "";
  const index = ws ? world.knowledge[ws]?.notes.find((n) => n.path === "INDEX.md")?.content ?? "" : "";
  const kb = ws
    ? `# Knowledge base\n\nYour memory is the Launchpad knowledge base of the workspace "${name}", in \`$LAUNCHPAD_KB\`. Your harness's own memory is off: do not write memory files anywhere else.\n\n- Read it as files: start from INDEX.md below, look up with \`rg\` in that folder or \`launchpad kb search <words>\`.\n- Write rarely: only a few durable, high-level findings, and what the owner asks you to remember. Never keep a log of sessions or runs.\n\n## INDEX.md\n\n${index.trimEnd()}\n`
    : "Your harness's own memory is off in Launchpad, and this session has no workspace, so no knowledge base: do not write memory files.\n";
  const memory = s.bot ? memoryParts(world.botMemory[s.bot.id] ?? {}) : { text: "", cut: null };
  const access = s.bot ? world.bots.find((b) => b.id === s.bot!.id)?.settings.launchpad ?? "propose" : "propose";
  const instructions: StartPart[] = [{ key: "kb", label: "Knowledge base and its INDEX.md", text: kb }];
  if (memory.text) instructions.push({ key: "memory", label: "Bot memory", text: memory.text });
  instructions.push({ key: "cli", label: "Launchpad CLI reference", text: `# Launchpad CLI\n\nYour access to the owner's Launchpad through \`launchpad\` (JSON output) is **${access}**. Read: \`launchpad folders list\`, \`launchpad sessions list\`, \`launchpad today\`…${access === "read" ? "" : " Propose with \`launchpad propose <launch|message|schedule|…> --title \"…\" --spec-file <file.json>\`: proposals are inert cards the owner starts with a click."}\n` });
  instructions.push({ key: "attribution", label: "Attribution", text: "Attribution: Launchpad ends what you publish through it with the line `_Written by Claude Code via [Launchpad](https://github.com/Wauplin/launchpad)_`. Do not add that line or another attribution line yourself. Bot channel and DM posts and other text inside Launchpad never get an attribution line.\n" });
  if (memory.cut) instructions.push({ key: "memory-cut", label: "Memory cut", text: memory.cut });
  // What a launch adds once the session has its id and folder (`Sessions::launch`), shortened for the demo.
  const prompt = promptParts(s, "");
  if (s.target.kind === "temp" && ws) prompt.push({ key: "temp", label: "Temporary folder", text: "Launchpad temporary folder: your working directory `<its temporary folder>` is temporary. Launchpad saves its files on Home at the end of every turn, so they follow this session to another machine." });
  prompt.push({ key: "publish", label: "GitHub suggestions and connections", text: "Agent-written GitHub comments and reviews must stay local. Use the launchpad skill and `launchpad suggest comment|review --repo <owner/repo> --number <n> --body-file <file> --session-id $LAUNCHPAD_SESSION_ID`. Never publish suggestions yourself; the owner edits and posts with a click." });
  return { instructions, prompt };
}

const slug = (text: string) => text.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "").split("-").slice(0, 5).join("-") || "task";
const titleOf = (text: string) => { const line = text.trim().split("\n")[0].trim(); return line.length > 80 ? `${line.slice(0, 77)}…` : line || "New agent"; };

/** Launches an agent: a new session that replays the closest prerecorded scenario. */
export function launch(input: LaunchSpec, id = demoId("demo-run"), scenarioId?: string): Session {
  ready();
  const spec: LaunchSpec = { ...defaultSpec(), ...input };
  if (!spec.text.trim()) throw new DemoError(400, "the prompt is empty");
  if (session(id)) return session(id)!;
  spec.agent_traces = topicContext(id, spec.topics, spec.agent_traces);
  const harness = spec.harness ?? "claude";
  const place = where(spec);
  let cwd = place.cwd;
  if (spec.worktree && place.git && place.projectId && (spec.target.kind === "project" || spec.target.kind === "predraft")) {
    const wt = taskWorktree(id, place.projectId, slug(spec.text), clock.now());
    world.agentWorktrees.push(wt);
    cwd = wt.root;
  }
  const s: Session = {
    ...demoSession({ id, title: titleOf(spec.text), project: place.projectId, harness, minutesAgo: 0, status: "starting", cwd, spec: { ...spec, harness, profile: spec.profile ?? PROFILE[harness] ?? null } }, world.events, []),
    model: spec.model ?? MODELS[harness] ?? null, effort: spec.effort, prompt: renderPrompt(spec, cwd), seen_at: clock.now(),
    usage: { context_tokens: 0, context_window: 200_000, input_tokens: 0, cached_tokens: 0, output_tokens: 0, cost_usd: harness === "opencode" ? 0 : null, cost_unavailable: false },
  };
  world.sessions = [s, ...world.sessions];
  publishSession(s);
  ready();
  appendEvent(id, { kind: "user_message", text: s.prompt, client_id: id, queued: false, images: [] });
  appendEvent(id, { kind: "notice", text: "Demo: this run replays a prerecorded scenario." });
  refreshCounts();
  setTimeout(() => { if (session(id)?.status === "starting") playScenario(id, scenarioId ?? pickScenario(spec.text)); }, 600);
  return session(id)!;
}

route("POST", "/api/sessions", (r) => launch(r.body as LaunchSpec, r.headers.get("Idempotency-Key") ?? undefined));

route("POST", "/api/launch/preview", (r): Preview => {
  const spec: LaunchSpec = { ...defaultSpec(), ...(r.body as LaunchSpec) };
  let place: Where;
  try { place = where(spec); } catch { place = { projectId: null, cwd: "", git: false }; }
  const cwd = spec.worktree && place.git && place.projectId ? `${ROOT}/.worktrees/${place.projectId}/${slug(spec.text)}` : place.cwd || null;
  return { title: titleOf(spec.text), prompt: spec.text.trim() ? renderPrompt(spec, cwd ?? "") : "", parts: spec.text.trim() ? promptParts(spec, cwd ?? "") : [], cwd, add_dirs: spec.context_projects.map((c) => world.projects.find((p) => p.id === c.project_id)?.path).filter((p): p is string => !!p) };
});

route("POST", "/api/launch/start-preview", (r): StartPreview => {
  const spec = r.body as LaunchSpec;
  if (!spec.harness) throw new DemoError(400, "pick a profile");
  return startPreview(spec);
});

// ---------------------------------------------------------------- composer drafts (revisions, 409 on stale saves)

route("GET", "/api/launch/draft", (r) => world.drafts[r.query.get("workspace") ?? ""] ?? null);
route("PUT", "/api/launch/draft", (r) => {
  const key = r.query.get("workspace") ?? "";
  const body = { ...(r.body as Record<string, unknown>) };
  const base = Number(body.base_revision ?? 0);
  const force = !!body.force;
  const client = (body.client as string | undefined) ?? null;
  delete body.base_revision; delete body.force; delete body.client;
  const current = world.drafts[key]?.revision ?? 0;
  if (!force && current !== base) {
    return new Response(JSON.stringify({ error: "Another window or device saved a newer draft.", revision: current }), { status: 409, headers: { "Content-Type": "application/json" } });
  }
  const revision = current + 1;
  world.drafts[key] = { ...body, revision };
  publish("drafts", "changed", { workspace: key || null, revision, client });
  return { ok: true, revision };
});

// ---------------------------------------------------------------- delayed launches and follow-ups

function readyAt(conditions: LaunchDelay[]): number | null {
  const times = conditions.map((c) => c.kind === "in" ? clock.now() + c.minutes * 60_000 : c.kind === "at" ? c.at : c.kind === "quota" ? quotaReset(c.harness, c.window) : null).filter((t): t is number => t != null);
  return times.length ? Math.max(...times) : null;
}

const publishSchedules = () => publish("schedules", "changed", null);

function schedule(spec: LaunchSpec, conditions: LaunchDelay[], resume: string | null, title: string): ScheduledLaunch {
  if (!conditions.length) throw new DemoError(400, "choose when to start");
  const l: ScheduledLaunch = { id: demoId("demo-sched"), spec, resume_session_id: resume, conditions, title, created_at: clock.now(), ready_at: readyAt(conditions), status: "queued", error: null };
  world.schedules = [...world.schedules, l];
  publishSchedules();
  return l;
}

route("GET", "/api/launch/schedules", () => world.schedules);
route("POST", "/api/launch/schedules", (r) => {
  const spec = { ...defaultSpec(), ...(r.body?.spec as LaunchSpec) };
  if (!spec.text.trim()) throw new DemoError(400, "the prompt is empty");
  return schedule(spec, r.body?.conditions ?? [], null, titleOf(spec.text));
});
route("PATCH", "/api/launch/schedules/:id", (r) => {
  const l = world.schedules.find((x) => x.id === r.params.id);
  if (!l || l.status === "starting") throw new DemoError(400, "this agent already started");
  if (l.resume_session_id) throw new DemoError(400, "a follow-up cannot be edited");
  const spec = { ...defaultSpec(), ...(r.body?.spec as LaunchSpec) };
  if (!spec.text.trim()) throw new DemoError(400, "the prompt is empty");
  const conditions: LaunchDelay[] = r.body?.conditions ?? l.conditions;
  if (!conditions.length) throw new DemoError(400, "choose when to start");
  const next: ScheduledLaunch = { ...l, spec, conditions, title: titleOf(spec.text), ready_at: readyAt(conditions), status: "queued", error: null };
  world.schedules = world.schedules.map((x) => (x.id === l.id ? next : x));
  publishSchedules();
  return next;
});
route("DELETE", "/api/launch/schedules/:id", (r) => {
  world.schedules = world.schedules.filter((l) => l.id !== r.params.id);
  publishSchedules();
  return { ok: true };
});
route("POST", "/api/launch/schedules/:id/start", (r) => {
  const l = world.schedules.find((x) => x.id === r.params.id);
  if (!l) throw new DemoError(404, "no such scheduled launch");
  return startSchedule(l);
});
route("POST", "/api/sessions/:id/follow-up", (r) => {
  const s = found(r.params.id);
  const message = String(r.body?.message ?? "").trim();
  if (!message) throw new DemoError(400, "the message is empty");
  return schedule({ ...s.launch_spec, text: message }, r.body?.conditions ?? [], s.id, `Follow-up: ${s.title}`);
});

function startSchedule(l: ScheduledLaunch): Session {
  world.schedules = world.schedules.filter((x) => x.id !== l.id);
  publishSchedules();
  if (l.resume_session_id) {
    const s = found(l.resume_session_id);
    sendMessage(s, l.spec.text, `${l.id}-msg`);
    return session(s.id)!;
  }
  return launch(l.spec);
}

const met = (l: ScheduledLaunch, c: LaunchDelay) => {
  switch (c.kind) {
    case "in": return clock.now() >= l.created_at + c.minutes * 60_000;
    case "at": return clock.now() >= c.at;
    case "quota": { const t = quotaReset(c.harness, c.window); return t != null && clock.now() >= t; }
    case "agents": return c.session_ids.every((id) => { const s = session(id); return !!s && (s.status === "waiting" || (s.status === "closed" && s.archived_done)); });
  }
};

/** Starts queued launches whose conditions are all met (checked every few seconds and after a clock fast-forward). */
function tick() {
  if (!world) return;
  for (const l of world.schedules) {
    if (l.status !== "queued" || !l.conditions.every((c) => met(l, c))) continue;
    if (l.resume_session_id) {
      const s = session(l.resume_session_id);
      if (!s || ["starting", "working", "in_terminal"].includes(s.status) || s.open_question || s.quota_pause) continue;
    }
    try { startSchedule(l); toast(`Started ${l.title}`); } catch (e) { l.status = "failed"; l.error = e instanceof Error ? e.message : String(e); publishSchedules(); }
  }
}
setInterval(tick, 5000);
onClock(tick);

// ---------------------------------------------------------------- crons

/** The next `count` times (epoch ms, local time) a 5-field cron expression fires. */
export function nextRuns(expr: string, count: number, from = clock.now()): number[] {
  const fields = expr.trim().split(/\s+/);
  if (fields.length !== 5) throw new DemoError(400, "use 5 fields: minute hour day-of-month month day-of-week");
  const ranges: [number, number][] = [[0, 59], [0, 23], [1, 31], [1, 12], [0, 7]];
  const sets = fields.map((f, i) => {
    const [lo, hi] = ranges[i];
    const out = new Set<number>();
    for (const part of f.split(",")) {
      const m = /^(\*|(\d+)(?:-(\d+))?)(?:\/(\d+))?$/.exec(part);
      if (!m) throw new DemoError(400, `invalid cron field: ${f}`);
      const a = m[1] === "*" ? lo : Number(m[2]);
      const b = m[1] === "*" ? hi : m[3] != null ? Number(m[3]) : m[4] ? hi : a;
      const step = Number(m[4] ?? 1);
      if (a < lo || b > hi || a > b || step < 1) throw new DemoError(400, `invalid cron field: ${f}`);
      for (let v = a; v <= b; v += step) out.add(i === 4 && v === 7 ? 0 : v);
    }
    return out;
  });
  const [min, hour, dom, mon, dow] = sets;
  const anyDom = fields[2] === "*", anyDow = fields[4] === "*";
  const d = new Date(from);
  d.setSeconds(0, 0);
  d.setMinutes(d.getMinutes() + 1);
  const result: number[] = [];
  for (let guard = 0; result.length < count && guard < 600_000; guard++) {
    const dayOk = anyDom && anyDow ? true : anyDom ? dow.has(d.getDay()) : anyDow ? dom.has(d.getDate()) : dom.has(d.getDate()) || dow.has(d.getDay());
    if (!mon.has(d.getMonth() + 1) || !dayOk) { d.setDate(d.getDate() + 1); d.setHours(0, 0, 0, 0); continue; }
    if (!hour.has(d.getHours())) { d.setHours(d.getHours() + 1, 0, 0, 0); continue; }
    if (min.has(d.getMinutes())) result.push(d.getTime());
    d.setMinutes(d.getMinutes() + 1);
  }
  return result;
}

const publishCrons = () => publish("crons", "changed", null);
const cronOf = (id: string) => { const c = world.crons.find((x) => x.id === id); if (!c) throw new DemoError(404, "no such cron"); return c; };

/** One change of a bot's files in its history (Home's plain history), with its diff. */
export function recordBotChange(botId: string, path: string, before: string | null, after: string | null) {
  const list = (world.botChanges[botId] ??= []);
  const id = Math.max(0, ...list.map((c) => c.id)) + 1;
  const lines = (t: string | null) => { const x = (t ?? "").trimEnd(); return x ? x.split("\n") : []; };
  const patch = `@@ -1 +1 @@\n${[...lines(before).map((l) => `-${l}`), ...lines(after).map((l) => `+${l}`)].join("\n")}\n`;
  list.unshift({ id, path, sha256: before != null ? `old-${id}` : null, new_sha256: after != null ? `new-${id}` : null, at: clock.now(), session_id: null, author: { kind: "owner", name: "You", session_id: null, model: null, runner: null }, added: lines(after).length, removed: lines(before).length, patch });
  const bot = world.bots.find((b) => b.id === botId);
  if (bot) { bot.version += 1; publish("bots", "changed", { id: botId }); }
}

/** A bot routine's file was written: the record follows it (Home's `crons::sync_bot_files`). */
export function applyRoutineFile(botId: string, path: string, text: string | null) {
  const c = world.crons.find((x) => x.bot_id === botId && x.file === path);
  const before = c ? routineToml(defOf(c)) : null;
  if (text == null) {
    if (c) world.crons = world.crons.filter((x) => x !== c);
  } else {
    let d;
    try { d = parseRoutineToml(text); } catch (e) { throw new DemoError(400, `${path}: ${e instanceof Error ? e.message : String(e)}`); }
    if (d.trigger.kind === "schedule") nextRuns(d.schedule, 1);
    if (c) {
      c.title = d.title; c.paused = d.paused; c.schedule = d.schedule; c.time_zone = d.time_zone; c.trigger = d.trigger; c.conditions = d.conditions; c.spec = { ...c.spec, text: d.instructions }; c.file_error = null;
      c.next_run_at = nextDue(c);
    } else {
      const bot = world.bots.find((b) => b.id === botId);
      if (!bot) throw new DemoError(400, "this bot no longer exists");
      const profile = world.config.profiles.find((x) => x.id === bot.settings.profile);
      createCron({ ...defaultSpec(), harness: profile?.harness ?? "claude", profile: bot.settings.profile ?? null, target: botTarget(bot), text: d.instructions, workspace_id: bot.workspace_id }, d.schedule, d.conditions, d.trigger, { title: d.title, paused: d.paused, bot_id: bot.id, bot_paused: bot.paused, file: path, ...(d.time_zone ? { time_zone: d.time_zone } : {}) });
    }
  }
  recordBotChange(botId, path, before, text);
  publishCrons();
}

export { routinePath };

export const nextDue = (c: CronAgent) => (c.paused || c.bot_paused || c.trigger.kind !== "schedule" ? null : nextRuns(c.schedule, 1)[0] ?? null);

export function createCron(spec: LaunchSpec, expr: string, conditions: CronCondition[], trigger: CronAgent["trigger"] = { kind: "schedule" }, extra: Partial<CronAgent> = {}): CronAgent {
  if (!spec.text.trim()) throw new DemoError(400, "the prompt is empty");
  if (trigger.kind === "schedule") nextRuns(expr, 1);
  if (trigger.kind === "github" && !trigger.events.length) throw new DemoError(400, "pick at least one kind of GitHub activity");
  if (trigger.kind === "failure" && !trigger.events.length) throw new DemoError(400, "pick at least one kind of failure");
  const c: CronAgent = { id: demoId("demo-cron"), spec, title: titleOf(spec.text), schedule: expr.trim(), trigger, bot_id: null, bot_paused: false, conditions, paused: false, created_at: clock.now(), next_run_at: null, last_check_at: null, last_outcome: null, runs: [], history: [], github_seen: null, failure_seen: null, channel_seen: null, file: null, file_error: null, waiting: null, ...extra };
  c.next_run_at = nextDue(c);
  world.crons = [...world.crons, c];
  publishCrons();
  return c;
}

route("GET", "/api/launch/crons", () => world.crons);
route("GET", "/api/launch/crons/next", (r) => nextRuns(r.query.get("schedule") ?? "", 3));
route("POST", "/api/launch/crons", (r) => createCron({ ...defaultSpec(), ...(r.body?.spec as LaunchSpec) }, String(r.body?.schedule ?? ""), r.body?.conditions ?? [], r.body?.trigger ?? { kind: "schedule" }, { ...(r.body?.title ? { title: String(r.body.title) } : {}), ...(r.body?.time_zone ? { time_zone: String(r.body.time_zone) } : {}) }));
route("PATCH", "/api/launch/crons/:id", (r) => {
  const c = cronOf(r.params.id);
  const before = c.file ? routineToml(defOf(c)) : null;
  if (r.body?.trigger) c.trigger = r.body.trigger;
  if (r.body?.schedule != null) c.schedule = String(r.body.schedule).trim();
  if (c.trigger.kind === "schedule") nextRuns(c.schedule, 1);
  if (r.body?.conditions) c.conditions = r.body.conditions;
  if (r.body?.paused != null) c.paused = !!r.body.paused;
  if (r.body?.time_zone) c.time_zone = String(r.body.time_zone);
  if (r.body?.title != null) c.title = String(r.body.title).trim() || c.title;
  if (r.body?.instructions != null) c.spec = { ...c.spec, text: String(r.body.instructions).trim() };
  c.next_run_at = nextDue(c);
  // A bot routine: saving writes its file, a version in the bot's history.
  if (c.file && c.bot_id) recordBotChange(c.bot_id, c.file, before, routineToml(defOf(c)));
  publishCrons();
  return c;
});
route("DELETE", "/api/launch/crons/:id", (r) => {
  const c = cronOf(r.params.id);
  world.crons = world.crons.filter((x) => x.id !== r.params.id);
  if (c.file && c.bot_id) recordBotChange(c.bot_id, c.file, routineToml(defOf(c)), null);
  publishCrons();
  return { ok: true };
});
route("POST", "/api/launch/crons/:id/run", (r) => {
  const c = cronOf(r.params.id);
  // A bot's routine posts to the bot's current session; others start a new agent.
  const thread = c.bot_id ? world.sessions.filter((s) => s.launch_spec.bot?.id === c.bot_id && s.status !== "closed").sort((a, b) => b.started_at - a.started_at)[0] : null;
  let s: Session;
  if (thread) {
    if (["starting", "working", "background"].includes(thread.status)) throw new DemoError(400, "The bot is working: wait for its turn to end, or Stop it");
    sendMessage(thread, routineText(c.title, "Run now", c.spec.text), demoId("demo-routine"));
    s = thread;
  } else s = launch(c.spec);
  c.runs = [...c.runs, s.id];
  c.history = [...c.history, { at: clock.now(), cause: "Run now", session_id: s.id, outcome: "running", detail: null }];
  c.last_check_at = clock.now();
  c.last_outcome = "Started";
  publishCrons();
  return s;
});

// ---------------------------------------------------------------- proposals

const publishProposal = (p: Proposal) => publish("proposals", "changed", p);
const proposalOf = (id: string) => { const p = world.proposals.find((x) => x.id === id); if (!p) throw new DemoError(404, "no such proposal"); return p; };

route("GET", "/api/proposals", (r) => {
  const w = r.query.get("workspace");
  if (!w || w === "all") return world.proposals;
  const ws = world.workspaces.find((x) => x.id === w || x.name.toLowerCase() === w.toLowerCase());
  if (!ws) throw new DemoError(404, `no workspace ${w}`);
  return world.proposals.filter((p) => p.workspace_id === ws.id);
});
route("POST", "/api/proposals", () => notInDemo());
route("POST", "/api/proposals/:id/accept", (r) => {
  const p = proposalOf(r.params.id);
  if (p.state !== "proposed" && p.state !== "failed") throw new DemoError(400, "This proposal was already handled.");
  const now = clock.now();
  try {
    const b = p.body;
    const spec = "spec" in b ? { ...b.spec, allow_paid_usage: false } : null;
    if (b.kind === "launch") p.result_id = launch(spec!, p.result_id ?? undefined).id;
    else if (b.kind === "schedule") p.result_id = schedule(spec!, b.conditions, null, p.title).id;
    else if (b.kind === "cron") p.result_id = createCron(spec!, b.schedule, b.conditions).id;
    else if (b.kind === "message") { sendMessage(found(b.session_id), b.text, `${p.id}-msg`); p.result_id = b.session_id; }
    else if (b.kind === "tool") p.result_id = b.tool.id;
    else if (b.kind === "channel") p.result_id = createChannel(p.workspace_id ?? "", b.name, b.purpose, b.members).id;
    else if (b.kind === "channel_join") {
      const c = world.botChannels.find((x) => x.id === b.channel);
      if (!c) throw new DemoError(400, "this channel no longer exists");
      if (c.archived) throw new DemoError(400, `#${c.name} is archived: bring it back first`);
      if (!c.members.some((m) => m.bot_id === b.bot_id)) setMember(c.id, b.bot_id, b.mode);
      p.result_id = c.id;
    }
    else if (b.kind === "bot_instructions") {
      const bot = world.bots.find((x) => x.id === b.bot_id);
      if (!bot) throw new DemoError(400, "this bot no longer exists");
      if (bot.instructions !== b.before) throw new DemoError(400, `BOT.md changed since this proposal: ask ${bot.settings.name} again`);
      bot.instructions = b.instructions; bot.version += 1; publish("bots", "changed", { id: bot.id });
      p.result_id = bot.id;
    }
    else if (b.kind === "bot_files") {
      const bot = world.bots.find((x) => x.id === b.bot_id);
      if (!bot) throw new DemoError(400, "this bot no longer exists");
      // Every routine file is checked before anything is written: all or nothing.
      for (const [path, text] of Object.entries(b.files)) if (path !== "BOT.md" && text != null) parseRoutineToml(text);
      for (const [path, text] of Object.entries(b.files)) {
        if (path === "BOT.md") { const before = bot.instructions; bot.instructions = text ?? before; recordBotChange(bot.id, path, before, bot.instructions); }
        else applyRoutineFile(bot.id, path, text ?? null);
      }
      p.result_id = bot.id;
    }
    else if (b.kind === "routine") {
      const bot = world.bots.find((x) => x.id === b.bot_id);
      if (!bot) throw new DemoError(400, "this bot no longer exists");
      const profile = world.config.profiles.find((x) => x.id === bot.settings.profile);
      p.result_id = createCron({ ...defaultSpec(), harness: profile?.harness ?? "claude", profile: bot.settings.profile ?? null, target: botTarget(bot), text: b.instructions, workspace_id: bot.workspace_id }, b.schedule, b.conditions, b.trigger, { title: b.title, bot_id: bot.id, bot_paused: bot.paused }).id;
    }
    p.state = "started";
    p.error = null;
  } catch (e) {
    p.state = "failed";
    p.error = e instanceof Error ? e.message : String(e);
  }
  p.updated_at = now;
  publishProposal(p);
  return p;
});
route("POST", "/api/proposals/:id/dismiss", (r) => {
  const p = proposalOf(r.params.id);
  p.state = "dismissed";
  p.updated_at = clock.now();
  publishProposal(p);
  return p;
});
// A reply to a report: the demo records it as answered; nothing is sent.
route("POST", "/api/proposals/:id/reply", (r) => {
  const p = proposalOf(r.params.id);
  p.state = "started";
  p.result_id = p.session_id;
  p.updated_at = clock.now();
  publishProposal(p);
  return p;
});
route("POST", "/api/proposals/:id/edit", (r) => {
  const p = proposalOf(r.params.id);
  p.state = "replaced";
  p.replaced_by = null;
  p.updated_at = clock.now();
  publishProposal(p);
  return p;
});
route("POST", "/api/proposals/:id/recheck", (r) => {
  const p = proposalOf(r.params.id);
  if (p.state === "uncertain") { p.state = p.result_id && session(p.result_id) ? "started" : "failed"; p.updated_at = clock.now(); publishProposal(p); }
  return p;
});

// ---------------------------------------------------------------- uploads (memory only)

const toDataUrl = (blob: Blob) => new Promise<string>((resolve, reject) => { const r = new FileReader(); r.onload = () => resolve(String(r.result)); r.onerror = () => reject(r.error); r.readAsDataURL(blob); });

route("POST", "/api/uploads", async (r) => {
  const body: unknown = r.body;
  const blob = body instanceof Blob ? body : new Blob([typeof body === "string" ? body : JSON.stringify(body ?? "")]);
  if (blob.size > 5 * 1024 * 1024) throw new DemoError(400, "attachments are limited to 5 MB each");
  const mime = r.headers.get("Content-Type") || blob.type || "application/octet-stream";
  const bytes = new Uint8Array(await blob.arrayBuffer());
  const digest = crypto.subtle ? new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)) : bytes.slice(0, 32);
  const hash = [...digest].map((b) => b.toString(16).padStart(2, "0")).join("");
  // Data URLs remain accepted everywhere an `upload:` reference is, and previews need no daemon.
  const data_url = await toDataUrl(new Blob([bytes], { type: mime }));
  world.uploads[hash] = { name: hash, data_url, blob };
  return { data_url };
});
route("GET", "/api/uploads/:hash", (r) => {
  const u = world.uploads[r.params.hash];
  if (!u) throw new DemoError(404, "no such upload");
  return new Response(u.blob, { headers: { "Content-Type": r.query.get("type") ?? u.blob.type } });
});

