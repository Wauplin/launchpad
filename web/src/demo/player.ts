// Scenario player: prerecorded agent runs (plan section 6). A scenario is a timeline of the same SessionEvent bodies a
// real harness produces; the player plays it into a session through the store (`appendEvent`, `setStatus`), so the
// unchanged Trace, Summary, Changes and agent list render it. Launch options select variants (`when`), answers pick
// branches (`ask`), approvals wait for the approval card (`approve`) and quota pauses wait for the demo clock.
//
// One interpreter serves two drivers: `prefill` runs a scenario synchronously into a world being built (up to its
// first question, approval or quota pause), and the live driver plays it with delays. A world session prefilled up to
// a wait point continues live when it is answered or when the clock passes its quota reset. Item and request IDs
// depend only on the scenario and the session's log length, so a scenario plays identically every time.

import type { EventBody } from "@/gen/EventBody";
import type { PermissionMode } from "@/gen/PermissionMode";
import type { PlanItem } from "@/gen/PlanItem";
import type { Proposal } from "@/gen/Proposal";
import type { Session } from "@/gen/Session";
import type { SessionEvent } from "@/gen/SessionEvent";
import type { SessionGitCommit } from "@/gen/SessionGitCommit";
import type { SessionGitFile } from "@/gen/SessionGitFile";
import type { SessionStatus } from "@/gen/SessionStatus";
import type { TaskWorktree } from "@/gen/TaskWorktree";
import { clock, onClock, publish } from "./daemon";
import { appendEvent, session, setStatus, updateSession } from "./store";
import { world } from "./world";

// ---------------------------------------------------------------- shared state shapes

/** Git fixture of one session (Changes tab): uncommitted or committed files, with one patch per file. */
export type GitFixture = {
  branch: string;
  files: SessionGitFile[];
  patches: Record<string, string>;
  commits: SessionGitCommit[];
  /** The changes are committed on the task branch (Changes shows Committed). */
  committed: boolean;
  merged_into: string | null;
};

/** What a scenario may touch: the agents part of the world (the live world, or the one being built). */
export type AgentsState = {
  sessions: Session[];
  events: Record<string, SessionEvent[]>;
  sessionGit: Record<string, GitFixture>;
  agentWorktrees: TaskWorktree[];
  proposals: Proposal[];
  /** Background command output (`/api/sessions/:id/task-output`), per session and task. */
  taskOutputs: Record<string, Record<string, string>>;
  /** World sessions prefilled from a scenario, continued live on answer, approval or quota reset. */
  dormant: Record<string, Dormant>;
};

export type Options = {
  permission: PermissionMode;
  /** When done: merge into main (`done:merge`) or keep the branch for review (`done:keep`). */
  merge: boolean;
  /** Replay the timeline forever (running world sessions). */
  loop?: boolean;
};

export type Dormant = { scenario: string; options: Options; prefix: string; skip: number };

type Ctx = { sessionId: string; scenarioId: string; harness: string; w: AgentsState; live: boolean; /** Replaying steps already in the world: no side effects. */ quiet?: boolean; options: Options; id: (kind?: string) => string };

// ---------------------------------------------------------------- builders

export type Op =
  | { t: "event"; delay: number; body: (ctx: Ctx) => EventBody }
  | { t: "tool"; delay: number; name: string; cmd: string; exit: number; output: string; duration: number; effect?: (ctx: Ctx) => void }
  | { t: "edit"; delay: number; path: string; diff: string; change: string }
  | { t: "say"; delay: number; text: string; final: boolean }
  | { t: "effect"; delay: number; fn: (ctx: Ctx) => void }
  | { t: "ask"; delay: number; question: string; header: string; options: { label: string; description: string }[] }
  | { t: "approve"; delay: number; title: string; cmd: string }
  | { t: "quota"; delay: number; minutes: number };

export type Item =
  | { k: "at"; delay: number; ops: Op[] }
  | { k: "ask"; delay: number; question: string; header: string; descriptions: Record<string, string>; branches: Record<string, Item[]> }
  | { k: "when"; cond: string; items: Item[] }
  | { k: "approve"; delay: number; cmd: string; title: string }
  | { k: "quota"; delay: number; minutes: number };

export type Scenario = {
  id: string;
  /** The folder and harness it was written for (informational: a launch keeps its own folder and harness). */
  folder: string;
  harness: string;
  title: string;
  timeline: Item[];
};

export const scenario = (s: Scenario): Scenario => s;

/** `delay` seconds after the previous step, run `action` (one op or several). Items pass through with the delay added. */
export function at(delay: number, action: Op | Op[] | Item): Item {
  if ("k" in action) return action.k === "when" ? action : { ...action, delay: action.delay + delay };
  return { k: "at", delay, ops: Array.isArray(action) ? action : [action] };
}

export const think = (text: string): Op => ({ t: "event", delay: 0, body: (c) => ({ kind: "reasoning", item_id: c.id("r"), text }) });
export const notice = (text: string): Op => ({ t: "event", delay: 0, body: () => ({ kind: "notice", text }) });
export const plan = (items: [string, "pending" | "in_progress" | "completed"][]): Op => ({ t: "event", delay: 0, body: () => ({ kind: "plan", items: items.map(([text, status]): PlanItem => ({ text, status })) }) });
/** A tool call with its result. `name` is generic (`bash`, `read`, `gh`); each harness shows its own tool name. */
export const tool = (name: string, cmd: string, o: { exit?: number; output?: string; seconds?: number; effect?: (ctx: Ctx) => void } = {}): Op =>
  ({ t: "tool", delay: 0, name, cmd, exit: o.exit ?? 0, output: o.output ?? "", duration: o.seconds ?? 1.2, effect: o.effect });
/** A file change: `diff` is one or more unified hunks (`@@ … @@` then ` `, `-`, `+` lines). */
export const edit = (path: string, diff: string, change = "update"): Op => ({ t: "edit", delay: 0, path, diff, change });
/** An intermediate assistant message (the turn continues). */
export const say = (text: string): Op => ({ t: "say", delay: 0, text, final: false });
/** A side effect on the world (Local PR hooks, proposals, worktree merges). */
export const effect = (fn: (ctx: Ctx) => void): Op => ({ t: "effect", delay: 0, fn });

/** The final message of the turn: streamed, then the turn ends and the session waits for input. */
export const final = (text: string): Item => ({ k: "at", delay: 1, ops: [{ t: "say", delay: 0, text, final: true }] });

/** A question with up to two answers; each answer continues with its own steps, then the main timeline. */
export function ask(question: string, branches: Record<string, Item[]>, o: { header?: string; descriptions?: Record<string, string> } = {}): Item {
  return { k: "ask", delay: 0.6, question, header: o.header ?? "Question", descriptions: o.descriptions ?? {}, branches };
}

/** Option variants: `permission:ask`, `permission:yolo`, `permission:auto`, `done:merge`, `done:keep`. */
export const when = (cond: string, ...items: Item[]): Item => ({ k: "when", cond, items });

/** An approval card for `cmd` (use inside `when("permission:ask", …)`). Deny ends the turn with an explanation. */
export const approve = (cmd: string, title = "Run command"): Item => ({ k: "approve", delay: 0.6, cmd, title });

/** Included quota runs out: the session is Quota paused until the demo clock passes the reset `minutes` later. */
export const quotaPause = (minutes: number): Item => ({ k: "quota", delay: 0.8, minutes });

/** The agent asks Launchpad to merge its branch (When done: merge), and the merge lands. */
export function merged(branch: string): Item {
  return { k: "at", delay: 0.8, ops: [
    tool("bash", "launchpad work finish", { output: `Merge requested for ${branch}. Launchpad merges when this turn ends.`, seconds: 0.6 }),
    { t: "effect", delay: 0.6, fn: (c) => mergeWorktree(c, branch) },
    { t: "event", delay: 0, body: (c) => ({ kind: "notice", text: `Launchpad merged ${c.w.sessionGit[c.sessionId]?.branch ?? branch} into main (fast-forward). The worktree is kept until cleanup.` }) },
  ] };
}

// ---------------------------------------------------------------- hooks for other areas

export type LocalPrRequest = { sessionId: string; scenarioId: string; projectId: string | null; cwd: string; branch: string; title: string; body: string; issue: number | null };
const localPrHooks = new Set<(r: LocalPrRequest) => void>();
/** The GitHub area registers how a scenario's `launchpad pr create` becomes a Local PR (and a suggestion). */
export function onLocalPr(fn: (r: LocalPrRequest) => void) { localPrHooks.add(fn); return () => localPrHooks.delete(fn); }
export function localPr(r: Omit<LocalPrRequest, "sessionId" | "scenarioId" | "cwd" | "projectId" | "branch">): Op {
  return effect((c) => {
    const s = c.w.sessions.find((x) => x.id === c.sessionId);
    const branch = c.w.sessionGit[c.sessionId]?.branch ?? "lp/local-pr";
    const req: LocalPrRequest = { ...r, sessionId: c.sessionId, scenarioId: c.scenarioId, projectId: s?.project_id ?? null, cwd: s?.cwd ?? "", branch };
    for (const fn of localPrHooks) { try { fn(req); } catch (e) { console.error("demo onLocalPr hook failed", e); } }
  });
}

export type ScenarioPhase = "started" | "question" | "approval" | "quota" | "finished";
export type ScenarioReport = { sessionId: string; scenarioId: string; phase: ScenarioPhase };
const reporters = new Set<(r: ScenarioReport) => void>();
/** Progress of live runs, for the tour engine (wait for a question, the end of a run…). */
export function onScenario(fn: (r: ScenarioReport) => void) { reporters.add(fn); return () => reporters.delete(fn); }
const report = (r: ScenarioReport) => { for (const fn of reporters) { try { fn(r); } catch (e) { console.error(e); } } };

// ---------------------------------------------------------------- registry

const registry = new Map<string, Scenario>();
export function registerScenarios(list: Scenario[]) { for (const s of list) registry.set(s.id, s); }
export const scenarioById = (id: string) => registry.get(id);

// ---------------------------------------------------------------- git and worktree effects

export function gitOf(c: { w: AgentsState; sessionId: string }): GitFixture {
  const s = c.w.sessions.find((x) => x.id === c.sessionId);
  const wt = c.w.agentWorktrees.find((t) => t.sessions.includes(c.sessionId));
  return (c.w.sessionGit[c.sessionId] ??= { branch: wt?.branch ?? (s?.cwd.includes("/.worktrees/") ? "lp/task" : "main"), files: [], patches: {}, commits: [], committed: false, merged_into: null });
}

const shortHash = (text: string) => {
  let h = 2166136261;
  for (let i = 0; i < text.length; i++) h = Math.imul(h ^ text.charCodeAt(i), 16777619);
  return (h >>> 0).toString(16).padStart(8, "0");
};

/** A full `git diff` patch of one file from its hunks. */
export function gitPatch(path: string, hunks: string, change = "update"): string {
  const head = change === "add" ? `new file mode 100644\nindex 0000000..${shortHash(hunks).slice(0, 7)}\n--- /dev/null` : `index ${shortHash(path).slice(0, 7)}..${shortHash(hunks).slice(0, 7)} 100644\n--- a/${path}`;
  return `diff --git a/${path} b/${path}\n${head}\n+++ b/${path}\n${hunks.replace(/\n$/, "")}\n`;
}

function recordEdit(c: Ctx, path: string, diff: string, change: string) {
  const g = gitOf(c);
  const known = g.files.find((f) => f.path === path);
  const previous = g.patches[path];
  if (previous?.includes(diff.replace(/\n$/, ""))) return;
  g.patches[path] = previous ? `${previous}${diff.replace(/\n$/, "")}\n` : gitPatch(path, diff, change);
  if (!known) g.files.push({ path, status: change === "add" ? "A " : " M", previous: null });
  g.committed = false;
  if (c.live) publish("worktrees", "changed", null);
}

/** `git commit`: the changes move to the task branch history. */
export function commit(subject: string): Op {
  return tool("bash", `git commit -am "${subject}"`, { seconds: 0.5, output: `[task ${shortHash(subject).slice(0, 7)}] ${subject}`, effect: (c) => {
    const g = gitOf(c);
    g.commits.push({ id: shortHash(`${c.sessionId}:${subject}`).repeat(5).slice(0, 40), subject });
    g.committed = true;
  } });
}

function mergeWorktree(c: Ctx, branch: string) {
  const g = gitOf(c);
  if (!g.commits.length) g.commits.push({ id: shortHash(`${c.sessionId}:merge`).repeat(5).slice(0, 40), subject: "Task changes" });
  g.committed = true;
  g.merged_into = "main";
  const wt = c.w.agentWorktrees.find((t) => t.sessions.includes(c.sessionId) || t.branch === branch);
  const merged = { base: wt?.base_commit ?? null, into: "main", head: g.commits.at(-1)!.id, at: clock.now() };
  if (wt) wt.merged = merged;
  if (c.live) {
    // The Files/worktree area keeps its own records of the same folders.
    const info = world.taskWorktrees?.find((i) => i.worktree.root === wt?.root || i.worktree.sessions.includes(c.sessionId));
    if (info) { info.worktree.merged = merged; info.ahead = 0; info.dirty_files = 0; }
    publish("worktrees", "changed", null);
  }
}

// ---------------------------------------------------------------- interpreter

type Pause = { sleep: number } | { wait: "answer"; request_id: string; permission: boolean; first: string } | { wait: "quota"; until: number };

/** The ops of a timeline for one set of options; answers come back through `next(answers)`. */
function* walk(items: Item[], opts: Options): Generator<Op, boolean, string[] | undefined> {
  for (const it of items) {
    if (it.k === "at") {
      let first = true;
      for (const op of it.ops) { yield { ...op, delay: op.delay + (first ? it.delay : 0) }; first = false; }
    } else if (it.k === "when") {
      const [what, value] = it.cond.split(":");
      const ok = what === "permission" ? ({ ask: "approvals", yolo: "yolo", auto: "auto" } as Record<string, string>)[value] === opts.permission
        : what === "done" ? (value === "merge") === opts.merge : false;
      if (ok && (yield* walk(it.items, opts))) return true;
    } else if (it.k === "ask") {
      const labels = Object.keys(it.branches);
      const answers = yield { t: "ask", delay: it.delay, question: it.question, header: it.header, options: labels.map((label) => ({ label, description: it.descriptions[label] ?? "" })) };
      const branch = it.branches[answers?.[0] ?? ""] ?? it.branches[labels[0]];
      if (yield* walk(branch, opts)) return true;
    } else if (it.k === "approve") {
      const answers = yield { t: "approve", delay: it.delay, title: it.title, cmd: it.cmd };
      if (!answers?.[0]?.startsWith("Allow")) {
        yield { t: "say", delay: 0.8, final: true, text: `You denied \`${it.cmd}\`, so I did not run it. The changes are in the worktree; run \`${it.cmd}\` yourself or send me a message to continue.` };
        return true;
      }
    } else if (it.k === "quota") {
      yield { t: "quota", delay: it.delay, minutes: it.minutes };
    }
  }
  return false;
}

/** How each harness names the generic tools. */
function toolName(harness: string, name: string): string {
  const map: Record<string, Record<string, string>> = {
    claude: { bash: "Bash", read: "Read", grep: "Grep", edit: "Edit", write: "Write" },
    codex: { bash: "Shell", read: "Shell", grep: "Shell", edit: "apply_patch", write: "apply_patch" },
    opencode: { bash: "bash", read: "read", grep: "grep", edit: "edit", write: "write" },
  };
  return map[harness]?.[name] ?? name;
}

type Out = {
  emit(body: EventBody): void;
  live(body: EventBody): void;
  status(status: SessionStatus, patch?: Partial<Session>): void;
  patch(p: Partial<Session>): void;
};

const permissionOptions = (harness: string) => harness === "codex"
  ? [{ label: "Allow once", description: "" }, { label: "Allow for session", description: "" }, { label: "Deny", description: "" }, { label: "Cancel turn", description: "" }]
  : [{ label: "Allow once", description: "Run this operation" }, { label: "Deny", description: "Do not run this operation" }];

/** Runs one op, yielding pauses (delays and waits). `resumed`: the op's wait was already emitted by `prefill`. */
function* exec(op: Op, c: Ctx, out: Out, resumed = false): Generator<Pause, string[] | undefined, string[] | undefined> {
  switch (op.t) {
    case "event": out.emit(op.body(c)); return;
    case "effect": if (!c.quiet) op.fn(c); return;
    case "tool": {
      const item_id = c.id("t");
      out.emit({ kind: "tool_call", item_id, name: toolName(c.harness, op.name), summary: op.cmd, input: op.name === "read" ? { file_path: op.cmd } : { command: op.cmd } });
      const lines = op.output.split("\n");
      const chunks = Math.min(4, lines.length);
      for (let i = 0; i < chunks && op.output; i++) {
        yield { sleep: op.duration / (chunks + 1) };
        const part = lines.slice(Math.floor((i * lines.length) / chunks), Math.floor(((i + 1) * lines.length) / chunks)).join("\n");
        out.live({ kind: "tool_output", item_id, text: part + (i < chunks - 1 ? "\n" : "") });
      }
      yield { sleep: op.duration / (chunks + 1) };
      if (!c.quiet) op.effect?.(c);
      out.emit({ kind: "tool_result", item_id, output: op.output, is_error: op.exit !== 0, exit_code: op.name === "bash" ? op.exit : null, duration_ms: Math.round(op.duration * 1000) });
      return;
    }
    case "edit": {
      const item_id = c.id("e");
      out.emit({ kind: "tool_call", item_id, name: toolName(c.harness, op.change === "add" ? "write" : "edit"), summary: op.path, input: { file_path: op.path } });
      yield { sleep: 0.4 };
      out.emit({ kind: "file_change", item_id, path: op.path, change: op.change, diff: op.diff.replace(/\n$/, "") });
      out.emit({ kind: "tool_result", item_id, output: `Updated ${op.path}`, is_error: false, exit_code: null, duration_ms: 400 });
      if (!c.quiet) recordEdit(c, op.path, op.diff, op.change);
      return;
    }
    case "say": {
      const item_id = c.id("m");
      const words = op.text.split(/(?<=\s)/);
      const steps = Math.min(12, Math.max(2, Math.ceil(words.length / 8)));
      for (let i = 0; i < steps; i++) {
        out.live({ kind: "assistant_delta", item_id, text: words.slice(Math.floor((i * words.length) / steps), Math.floor(((i + 1) * words.length) / steps)).join("") });
        yield { sleep: 0.12 };
      }
      out.emit({ kind: "assistant_message", item_id, text: op.text });
      if (op.final) {
        out.emit({ kind: "turn_end", duration_ms: 20_000 + op.text.length * 10, interrupted: false, error: null });
        out.status("waiting");
      }
      return;
    }
    case "ask":
    case "approve": {
      const permission = op.t === "approve";
      const request_id = c.id("q");
      const options = permission ? permissionOptions(c.harness) : op.options;
      if (!resumed) {
        const question = permission ? `${op.title}\n${JSON.stringify({ command: op.cmd, cwd: c.w.sessions.find((s) => s.id === c.sessionId)?.cwd ?? "" }, null, 2)}` : op.question;
        out.emit({ kind: "ask_user", request_id, questions: [{ permission, id: request_id, header: permission ? "Approval required" : op.header, question, options, multi_select: false }] });
        out.patch({ open_question: true });
        out.status("waiting");
      }
      const answers = yield { wait: "answer", request_id, permission, first: options[0].label };
      out.emit({ kind: "ask_answered", request_id, answers: answers ?? [options[0].label] });
      out.patch({ open_question: false });
      out.status("working");
      return answers ?? [options[0].label];
    }
    case "quota": {
      const until = clock.now() + op.minutes * 60_000;
      if (!resumed) {
        out.emit({ kind: "notice", text: "Included quota exhausted. Pausing until quota is available; interruption is best effort and commands may still finish." });
        out.emit({ kind: "turn_end", duration_ms: null, interrupted: true, error: null });
        out.status("quota_paused", { quota_pause: { phase: "paused", blockers: [{ window: "5-hour", resets_at: Math.floor(until / 1000) }], message: "Continue the unfinished turn.", images: [], client_id: null, error: null, messages: [] } });
      }
      const s = c.w.sessions.find((x) => x.id === c.sessionId);
      const reset = s?.quota_pause?.blockers[0]?.resets_at;
      yield { wait: "quota", until: reset != null ? reset * 1000 : until };
      for (const message of s?.quota_pause?.messages ?? []) out.emit({ kind: "message_delivered", client_id: message.client_id });
      out.patch({ quota_pause: null });
      out.emit({ kind: "notice", text: "Fresh quota available in both windows. Continuing the paused turn." });
      out.emit({ kind: "user_message", text: "Continue the unfinished turn.", client_id: c.id("c"), queued: false, images: [] });
      out.status("working");
      return;
    }
  }
}

function ctxFor(w: AgentsState, sessionId: string, scenarioId: string, options: Options, prefix: string, live: boolean): Ctx {
  const s = w.sessions.find((x) => x.id === sessionId);
  let n = 0;
  return { sessionId, scenarioId, harness: s?.harness ?? "claude", w, live, options, id: (kind = "i") => `${prefix}-${kind}${++n}` };
}

/**
 * Plays `scenarioId` synchronously into a session of a world being built, up to its first wait point (question,
 * approval, quota pause) or its end. Returns the event bodies and the session fields to use; a wait point is
 * remembered in `w.dormant` so the session continues live later.
 */
export function prefill(w: AgentsState, sessionId: string, scenarioId: string, options: Options): { events: EventBody[]; patch: Partial<Session> } {
  const sc = registry.get(scenarioId);
  if (!sc) throw new Error(`unknown scenario ${scenarioId}`);
  const prefix = `${scenarioId}-p`;
  const events: EventBody[] = [];
  const patch: Partial<Session> = {};
  const out: Out = { emit: (b) => events.push(b), live: () => {}, status: (status, p) => Object.assign(patch, { status, ...p }), patch: (p) => Object.assign(patch, p) };
  const c = ctxFor(w, sessionId, scenarioId, options, prefix, false);
  const gen = walk(sc.timeline, options);
  let skip = 0;
  for (let r = gen.next(); !r.done; r = gen.next(undefined), skip++) {
    const step = exec(r.value, c, out);
    for (let p = step.next(); !p.done; p = step.next(undefined)) {
      if ("wait" in p.value) {
        w.dormant[sessionId] = { scenario: scenarioId, options, prefix, skip };
        return { events, patch };
      }
    }
    if (options.loop && skip > 40) break;
  }
  if (options.loop) w.dormant[sessionId] = { scenario: scenarioId, options, prefix, skip: -1 };
  return { events, patch };
}

// ---------------------------------------------------------------- live driver

type Run = {
  sessionId: string;
  scenarioId: string;
  options: Options;
  world: unknown;
  skipping: boolean;
  aborted: boolean;
  wake: (() => void) | null;
  waiting: { kind: "answer"; request_id: string; first: string; resolve: (a: string[] | undefined) => void } | { kind: "quota"; until: number; resolve: () => void } | null;
};

const runs = new Map<string, Run>();
let speed = 1;
/** Playback speed of every run (1× or 2×). */
export function setSpeed(n: 1 | 2) { speed = n; }
export const getSpeed = () => speed;

const liveOut = (id: string): Out => ({
  emit: (b) => { appendEvent(id, b); },
  live: (b) => { appendEvent(id, b, true); },
  status: (status, p) => setStatus(id, status, p),
  patch: (p) => updateSession(id, p),
});

function sleep(run: Run, seconds: number): Promise<void> {
  if (run.skipping || run.aborted || seconds <= 0) return Promise.resolve();
  return new Promise((resolve) => {
    const t = setTimeout(() => { run.wake = null; resolve(); }, (seconds * 1000) / speed);
    run.wake = () => { clearTimeout(t); run.wake = null; resolve(); };
  });
}

const alive = (run: Run) => !run.aborted && run.world === world && runs.get(run.sessionId) === run;

async function drive(run: Run, resumeAt: Dormant | null) {
  const sc = registry.get(run.scenarioId)!;
  const s = session(run.sessionId);
  if (!s) return;
  const prefix = resumeAt?.prefix ?? `${run.scenarioId}-${s.last_seq}`;
  const out = liveOut(run.sessionId);
  report({ sessionId: run.sessionId, scenarioId: run.scenarioId, phase: "started" });
  let firstPass = true;
  do {
    const c = ctxFor(world, run.sessionId, run.scenarioId, run.options, firstPass ? prefix : `${run.scenarioId}-${session(run.sessionId)?.last_seq ?? 0}`, true);
    const gen = walk(sc.timeline, run.options);
    let skip = firstPass && resumeAt ? resumeAt.skip : -1;
    let r = gen.next();
    // A dormant session already shows the steps before its wait point: replay them silently to rebuild IDs.
    const silent: Out = { emit: () => {}, live: () => {}, status: () => {}, patch: () => {} };
    c.quiet = true;
    for (; skip > 0 && !r.done; skip--) {
      const step = exec(r.value, c, silent);
      for (let p = step.next(); !p.done; p = step.next(undefined)) { /* No effects: already in the world. */ }
      r = gen.next(undefined);
    }
    c.quiet = false;
    let resumed = skip === 0;
    if (!resumed && session(run.sessionId)?.status !== "working") setStatus(run.sessionId, "working");
    while (!r.done) {
      const op = r.value;
      await sleep(run, op.delay);
      if (!alive(run)) return;
      const step = exec(op, c, out, resumed);
      resumed = false;
      let p = step.next();
      while (!p.done) {
        const pause = p.value;
        let input: string[] | undefined;
        if ("sleep" in pause) await sleep(run, pause.sleep);
        else if (pause.wait === "answer") {
          report({ sessionId: run.sessionId, scenarioId: run.scenarioId, phase: pause.permission ? "approval" : "question" });
          input = run.skipping ? [pause.first] : await new Promise<string[] | undefined>((resolve) => { run.waiting = { kind: "answer", request_id: pause.request_id, first: pause.first, resolve }; });
          run.waiting = null;
        } else {
          report({ sessionId: run.sessionId, scenarioId: run.scenarioId, phase: "quota" });
          if (!run.skipping && clock.now() < pause.until) await new Promise<void>((resolve) => { run.waiting = { kind: "quota", until: pause.until, resolve }; });
          run.waiting = null;
        }
        if (!alive(run)) return;
        p = step.next(input);
      }
      r = gen.next(p.value);
    }
    firstPass = false;
    if (run.options.loop) await sleep(run, 2);
  } while (run.options.loop && alive(run));
  if (!alive(run)) return;
  runs.delete(run.sessionId);
  delete world.dormant[run.sessionId];
  report({ sessionId: run.sessionId, scenarioId: run.scenarioId, phase: "finished" });
}

function start(sessionId: string, scenarioId: string, options: Options, resumeAt: Dormant | null) {
  stopRun(sessionId);
  const run: Run = { sessionId, scenarioId, options, world, skipping: false, aborted: false, wake: null, waiting: null };
  runs.set(sessionId, run);
  void drive(run, resumeAt).catch((e) => console.error("demo scenario failed", scenarioId, e));
  return run;
}

/** Plays a scenario in a session (the session should exist; its harness names the tools). */
export function playScenario(sessionId: string, scenarioId: string, options: Partial<Options> = {}) {
  const s = session(sessionId);
  const full: Options = { permission: s?.launch_spec.permission_mode ?? "yolo", merge: !!s?.launch_spec.worktree?.merge_when_done, ...options };
  if (!registry.has(scenarioId)) throw new Error(`unknown scenario ${scenarioId}`);
  delete world.dormant[sessionId];
  start(sessionId, scenarioId, full, null);
}

/** The live run of a session, continuing a prefilled world session if needed. */
function runOf(sessionId: string): Run | null {
  const run = runs.get(sessionId);
  if (run && alive(run)) return run;
  const d = world.dormant[sessionId];
  if (!d || d.skip < 0) return null;
  return start(sessionId, d.scenario, d.options, d);
}

/** An answer or approval decision (`POST /api/sessions/:id/answer`). False when no scenario waits for it. */
export function answerRun(sessionId: string, request_id: string, answers: string[]): boolean {
  const run = runOf(sessionId);
  if (!run) return false;
  // A run continued from the world needs a moment to reach its wait point.
  const deliver = (tries: number) => {
    const w = run.waiting;
    if (w?.kind === "answer" && w.request_id === request_id) { w.resolve(answers); return; }
    if (tries > 0 && alive(run)) setTimeout(() => deliver(tries - 1), 20);
  };
  deliver(50);
  return true;
}

/** Finishes a run at once: no delays, questions take their first answer, approvals Allow once, quota waits end. */
export function skipToEnd(sessionId: string) {
  const run = runOf(sessionId);
  if (!run) return;
  run.skipping = true;
  run.wake?.();
  const settle = (tries: number) => {
    const w = run.waiting;
    if (w?.kind === "answer") w.resolve([w.first]);
    else if (w?.kind === "quota") w.resolve();
    else if (tries > 0 && alive(run)) setTimeout(() => settle(tries - 1), 20);
  };
  settle(50);
}

/** Stops a run (Stop, Archive). True when one was playing. */
export function stopRun(sessionId: string): boolean {
  const run = runs.get(sessionId);
  runs.delete(sessionId);
  if (!run) return false;
  run.aborted = true;
  run.wake?.();
  if (run.waiting?.kind === "answer") run.waiting.resolve(undefined);
  else if (run.waiting?.kind === "quota") run.waiting.resolve();
  return true;
}

/** Whether a scenario plays in the session now. */
export const isPlaying = (sessionId: string) => { const r = runs.get(sessionId); return !!r && alive(r); };

/** Permission or When done changed on a session: later `when` steps follow the new value. */
export function updateRunOptions(sessionId: string, patch: Partial<Options>) {
  const run = runs.get(sessionId);
  if (run) Object.assign(run.options, patch);
  const d = world.dormant[sessionId];
  if (d) d.options = { ...d.options, ...patch };
}

/** The quota reset time a paused run waits for, if any. */
export const quotaWaitOf = (sessionId: string) => { const w = runs.get(sessionId)?.waiting; return w?.kind === "quota" ? w.until : null; };

/** Continues a quota-paused run now (Resume with paid usage, Retry). */
export function continueQuota(sessionId: string): boolean {
  const run = runOf(sessionId);
  if (!run) return false;
  const settle = (tries: number) => {
    if (run.waiting?.kind === "quota") run.waiting.resolve();
    else if (tries > 0 && alive(run)) setTimeout(() => settle(tries - 1), 20);
  };
  settle(50);
  return true;
}

// ---------------------------------------------------------------- ambient runs and the clock

let ambientWorld: unknown = null;
/** Starts the looping replays of running world sessions once per world (called by the session routes). */
export function ensureAmbient() {
  if (!world || ambientWorld === world) return;
  ambientWorld = world;
  for (const [id, run] of runs) if (run.world !== world) { run.aborted = true; run.wake?.(); runs.delete(id); }
  for (const [id, d] of Object.entries(world.dormant)) if (d.options.loop) start(id, d.scenario, d.options, null);
  checkClock();
}

/** Quota-paused sessions continue once the demo clock passes their reset. */
function checkClock() {
  if (!world) return;
  for (const run of runs.values()) if (run.waiting?.kind === "quota" && clock.now() >= run.waiting.until) run.waiting.resolve();
  for (const s of world.sessions) {
    const reset = s.quota_pause?.blockers[0]?.resets_at;
    if (s.status === "quota_paused" && reset != null && clock.now() >= reset * 1000 && !runs.has(s.id) && world.dormant[s.id]) continueQuota(s.id);
  }
}
onClock(checkClock);
