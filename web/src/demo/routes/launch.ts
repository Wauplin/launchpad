// Launching agents: the composer draft, preview and launch, delayed launches and follow-ups, crons, Ask Launchpad
// (assistant threads and proposals) and uploads. A launch plays the prerecorded scenario closest to its prompt.

import type { CronAgent } from "@/gen/CronAgent";
import type { CronCondition } from "@/gen/CronCondition";
import type { LaunchDelay } from "@/gen/LaunchDelay";
import type { LaunchSpec } from "@/gen/LaunchSpec";
import type { Preview } from "@/gen/Preview";
import type { Proposal } from "@/gen/Proposal";
import type { ScheduledLaunch } from "@/gen/ScheduledLaunch";
import type { Session } from "@/gen/Session";
import { toast } from "@/components/Toast";
import { clock, DemoError, notInDemo, onClock, publish, route } from "../daemon";
import { playScenario } from "../player";
import { pickScenario } from "../scenarios";
import { appendEvent, defaultSpec, demoId, publishSession, refreshCounts, session, setStatus } from "../store";
import { world } from "../world";
import { demoSession, FOLDER, MODELS, PROFILE, SUMMARY_INSTRUCTION, taskWorktree } from "../world/agents";
import { P, ROOT } from "../world/ids";
import { found, quotaReset, ready, sendMessage } from "./agents";

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
      return { projectId: p.project_id, cwd: p.root, git: true };
    }
    case "session_folder": {
      const s = found(t.session_id);
      return { projectId: s.project_id, cwd: s.cwd, git: !!world.sessionGit[s.id] || !!s.project_id };
    }
    case "temp": return { projectId: P.scratch, cwd: `${ROOT}/${FOLDER[P.scratch]}`, git: false };
    case "browser": return notInDemo();
    case "assistant": throw new DemoError(400, "Ask Launchpad threads start from the Ask page.");
    default: throw new DemoError(400, "Choose a folder first.");
  }
}

/** `compose::render_prompt`: the typed text first, then context, presets and the Launchpad final-message block. */
function renderPrompt(spec: LaunchSpec, cwd: string): string {
  const parts = [spec.text.trim()];
  const context: string[] = [];
  for (const r of spec.github_refs) context.push(`- ${r.kind === "pr" ? "Pull request" : "Issue"} ${r.repo}#${r.number ?? ""}${r.title ? `: ${r.title}` : ""} (${r.url})`);
  for (const c of spec.context_projects) { const p = world.projects.find((x) => x.id === c.project_id); if (p) context.push(`- ${c.editable ? "Editable" : "Reference"} project: ${p.path}${p.description ? ` (${p.description})` : ""}.`); }
  for (const f of spec.files) context.push(`- File: ${f.root}/${f.path}${f.start_line ? ` lines ${f.start_line}-${f.end_line ?? f.start_line}` : ""}`);
  for (const a of spec.attachments) context.push(`- Attached ${a.data_url.startsWith("data:image/") || a.data_url.startsWith("upload:image/") ? "image" : "file (local copy provided at launch)"}: ${a.name}`);
  for (const t of spec.agent_traces) context.push(`- Trace of another agent: run \`launchpad trace ${t.session_id} --through ${t.through_seq}\` to read it.`);
  if (context.length) parts.push(`Context:\n${context.join("\n")}${spec.github_refs.length ? "\nUse `gh` to read issues, pull requests and comments." : ""}`);
  const presets = world.config.presets.filter((p) => (spec.target.kind === "assistant" ? p.ask_launchpad : p.agent) && spec.presets.includes(p.id) && p.text.trim()).map((p) => p.text.trim()).join(" ");
  if (presets) parts.push(presets);
  if (spec.worktree) parts.push(`Launchpad worktree: you work in ${cwd}, a Git worktree on its own branch. Commit your changes there.${spec.worktree.merge_when_done ? " When the task is done, run `launchpad work finish` so Launchpad merges the branch." : ""}`);
  parts.push(SUMMARY_INSTRUCTION);
  return parts.filter(Boolean).join("\n\n");
}

const slug = (text: string) => text.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "").split("-").slice(0, 5).join("-") || "task";
const titleOf = (text: string) => { const line = text.trim().split("\n")[0].trim(); return line.length > 80 ? `${line.slice(0, 77)}…` : line || "New agent"; };

/** Launches an agent: a new session that replays the closest prerecorded scenario. */
export function launch(input: LaunchSpec, id = demoId("demo-run"), scenarioId?: string): Session {
  ready();
  const spec: LaunchSpec = { ...defaultSpec(), ...input };
  if (!spec.text.trim()) throw new DemoError(400, "the prompt is empty");
  if (session(id)) return session(id)!;
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
  // Launching from a workspace window adds the folder to that workspace.
  const ws = world.workspaces.find((w) => w.id === spec.workspace_id);
  if (ws && place.projectId && !ws.project_ids.includes(place.projectId) && place.projectId !== P.scratch) {
    ws.project_ids = [...ws.project_ids, place.projectId];
    publish("workspaces", "changed", null);
  }
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
  return { title: titleOf(spec.text), prompt: spec.text.trim() ? renderPrompt(spec, cwd ?? "") : "", cwd, add_dirs: spec.context_projects.map((c) => world.projects.find((p) => p.id === c.project_id)?.path).filter((p): p is string => !!p) };
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

function createCron(spec: LaunchSpec, expr: string, conditions: CronCondition[]): CronAgent {
  if (!spec.text.trim()) throw new DemoError(400, "the prompt is empty");
  const c: CronAgent = { id: demoId("demo-cron"), spec, title: titleOf(spec.text), schedule: expr.trim(), conditions, paused: false, created_at: clock.now(), next_run_at: nextRuns(expr, 1)[0] ?? null, last_check_at: null, last_outcome: null, runs: [] };
  world.crons = [...world.crons, c];
  publishCrons();
  return c;
}

route("GET", "/api/launch/crons", () => world.crons);
route("GET", "/api/launch/crons/next", (r) => nextRuns(r.query.get("schedule") ?? "", 3));
route("POST", "/api/launch/crons", (r) => createCron({ ...defaultSpec(), ...(r.body?.spec as LaunchSpec) }, String(r.body?.schedule ?? ""), r.body?.conditions ?? []));
route("PATCH", "/api/launch/crons/:id", (r) => {
  const c = cronOf(r.params.id);
  if (r.body?.schedule != null) { nextRuns(r.body.schedule, 1); c.schedule = String(r.body.schedule).trim(); }
  if (r.body?.conditions) c.conditions = r.body.conditions;
  if (r.body?.paused != null) c.paused = !!r.body.paused;
  c.next_run_at = c.paused ? null : nextRuns(c.schedule, 1)[0] ?? null;
  publishCrons();
  return c;
});
route("DELETE", "/api/launch/crons/:id", (r) => { cronOf(r.params.id); world.crons = world.crons.filter((c) => c.id !== r.params.id); publishCrons(); return { ok: true }; });
route("POST", "/api/launch/crons/:id/run", (r) => {
  const c = cronOf(r.params.id);
  const s = launch(c.spec);
  c.runs = [...c.runs, s.id];
  c.last_check_at = clock.now();
  c.last_outcome = `Started ${c.title}`;
  publishCrons();
  return s;
});

// ---------------------------------------------------------------- Ask Launchpad

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
route("POST", "/api/proposals/:id/recheck", (r) => {
  const p = proposalOf(r.params.id);
  if (p.state === "uncertain") { p.state = p.result_id && session(p.result_id) ? "started" : "failed"; p.updated_at = clock.now(); publishProposal(p); }
  return p;
});

const assistantThread = (workspace: string | null) => world.sessions.filter((s) => s.launch_spec.target.kind === "assistant" && (s.launch_spec.target.workspace_id ?? null) === workspace && s.status !== "closed").sort((a, b) => b.started_at - a.started_at)[0] ?? null;

route("GET", "/api/assistant", (r) => { const w = r.query.get("workspace"); return assistantThread(w && w !== "all" ? w : null); });
route("POST", "/api/assistant", (r) => {
  ready();
  const workspace: string | null = r.body?.workspace_id ?? null;
  const text = String(r.body?.text ?? "").trim();
  let s = assistantThread(workspace);
  if (!s) {
    const profileId = world.config.assistant.profile ?? world.config.profiles.find((p) => p.billing === "subscription")?.id ?? world.config.profiles[0]?.id ?? "claude-max";
    const profile = world.config.profiles.find((p) => p.id === profileId);
    const harness = profile?.harness ?? "claude";
    const id = demoId("demo-ask");
    const name = world.workspaces.find((w) => w.id === workspace)?.name ?? "Everything";
    s = {
      ...demoSession({ id, title: `Ask Launchpad · ${name}`, project: null, harness, minutesAgo: 0, cwd: `/home/demo/.local/share/launchpad/assistant/${workspace ?? "all"}`,
        spec: { harness, profile: profileId, target: { kind: "assistant", workspace_id: workspace }, workspace_id: workspace, text, model: world.config.assistant.model } }, world.events, []),
      model: world.config.assistant.model ?? MODELS[harness] ?? null, seen_at: clock.now(),
    };
    world.sessions = [s, ...world.sessions];
    publishSession(s);
    if (!text) setStatus(s.id, "waiting");
  }
  if (text) {
    const attachments = (r.body?.attachments ?? []) as { name: string }[];
    sendMessage(s, text, `${s.id}-${s.last_seq + 1}`, attachments);
  }
  return session(s.id);
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

