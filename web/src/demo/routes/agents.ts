// Sessions, their events and the agent actions (`/api/sessions/*`). Messages and answers continue prerecorded
// scenarios (`player.ts`); every mutation publishes the same bus events as the daemon (`sessions`, `session/<id>`).

import type { MessageContext } from "@/gen/MessageContext";
import type { EarlierFacts } from "@/gen/EarlierFacts";
import type { PermissionMode } from "@/gen/PermissionMode";
import type { Session } from "@/gen/Session";
import type { SessionEvent } from "@/gen/SessionEvent";
import type { SessionEventPage } from "@/gen/SessionEventPage";
import type { SessionGitDiff } from "@/gen/SessionGitDiff";
import type { SessionGitHistory } from "@/gen/SessionGitHistory";
import type { SessionGitStatus } from "@/gen/SessionGitStatus";
import type { SessionProcess } from "@/gen/SessionProcess";
import type { WorktreeInfo } from "@/gen/WorktreeInfo";
import { clock, DemoError, publish, route } from "../daemon";
import { answerRun, continueQuota, ensureAmbient, isPlaying, playScenario, stopRun, updateRunOptions } from "../player";
import { pickReply } from "../scenarios";
import { appendEvent, light, refreshCounts, session, setStatus, updateSession } from "../store";
import { world } from "../world";
import { FOLDER } from "../world/agents";
import { localPrHooks } from "../world/github";
import { openDemoTerminal, removeDemoTerminal } from "./terminals";

export const found = (id: string): Session => {
  const s = session(id);
  if (!s) throw new DemoError(404, "no such session");
  return s;
};

/**
 * Called by the session and launch routes: starts the looping runs of the world once, and keeps the Files/worktree
 * area (`world.taskWorktrees`, `world.codeRoots`) aware of the agents' task worktrees.
 */
export function ready() {
  ensureAmbient();
  syncWorktrees();
  alignQuotaPauses();
}

/** The reported reset of a quota window, or the paused demo session's reset. */
export function quotaReset(harness: string, window: string, fallback = true): number | null {
  const q = Object.values(world.status.quotas ?? {}).find((x) => x && "harness" in x && x.harness === harness);
  const w = q ? (window === "weekly" ? q.weekly : q.five_hour) : null;
  if (w?.resets_at) return w.resets_at < 1e12 ? w.resets_at * 1000 : w.resets_at;
  if (!fallback) return null;
  const r = world.sessions.find((s) => s.harness === harness && s.quota_pause)?.quota_pause?.blockers[0]?.resets_at;
  return r != null ? r * 1000 : null;
}


/** Quota-paused sessions wait for the reset the status bar shows (the settings area owns the readings). */
function alignQuotaPauses() {
  for (const s of world.sessions) {
    const b = s.quota_pause?.blockers[0];
    if (!b || !world.dormant[s.id]) continue;
    const reset = quotaReset(s.harness, b.window === "weekly" ? "weekly" : "five_hour", false);
    if (reset && reset > clock.now() && Math.floor(reset / 1000) !== b.resets_at) updateSession(s.id, { quota_pause: { ...s.quota_pause!, blockers: [{ ...b, resets_at: Math.floor(reset / 1000) }] } });
  }
}

let lastSync = "";
function syncWorktrees() {
  const list: WorktreeInfo[] | undefined = world.taskWorktrees;
  if (!list) return;
  const key = `${world.agentWorktrees.length}:${list.length}`;
  if (key === lastSync && list === syncedList) return;
  for (const t of world.agentWorktrees) {
    const known = list.find((i) => i.worktree.root === t.root);
    if (known) {
      // The code area owns this folder: share its branch and record the session in it.
      for (const id of t.sessions) if (!known.worktree.sessions.includes(id)) known.worktree.sessions.push(id);
      for (const id of t.sessions) if (world.sessionGit[id]) world.sessionGit[id].branch = known.worktree.branch;
      continue;
    }
    const g = world.sessionGit[t.sessions[0]];
    list.push({ worktree: t, project_name: FOLDER[t.project_id] ?? t.project_id, path: t.root, exists: true, dirty_files: g && !g.committed ? g.files.length : 0, ahead: g?.commits.length ?? 0, behind: 0, target_checkout: t.repo });
    world.codeRoots[t.root] ??= { projectId: t.project_id, branch: t.branch, base: "main", overrides: {}, exists: true };
  }
  lastSync = `${world.agentWorktrees.length}:${list.length}`;
  syncedList = list;
}
let syncedList: unknown = null;

const visible = (s: Session, assistant: boolean) => assistant || s.launch_spec.target.kind !== "assistant";
const isAssistant = (s: Session) => s.launch_spec.target.kind === "assistant";

// ---------------------------------------------------------------- reading

route("GET", "/api/sessions", (r) => { ready(); return world.sessions.filter((s) => visible(s, r.query.get("include_assistant") === "true")).map(light); });
route("GET", "/api/sessions/search", (r) => {
  const q = (r.query.get("q") ?? "").trim().toLowerCase();
  if (!q) return [];
  const assistant = r.query.get("include_assistant") === "true";
  return world.sessions.filter((s) => visible(s, assistant) && (world.events[s.id] ?? []).some((e) => "text" in e.body && typeof e.body.text === "string" && e.body.text.toLowerCase().includes(q))).map((s) => s.id);
});
route("GET", "/api/sessions/:id", (r) => { ready(); return found(r.params.id); });
route("GET", "/api/sessions/:id/events", (r) => (world.events[r.params.id] ?? []).filter((e) => e.seq > Number(r.query.get("after") ?? 0)));

/** `session_pages::page`: the events before `before` (or the tail), with the facts of the earlier ones. */
route("GET", "/api/sessions/:id/events/page", (r): SessionEventPage => {
  ready();
  const log = world.events[r.params.id] ?? [];
  const before = r.query.get("before");
  const limit = Number(r.query.get("limit") ?? r.query.get("tail") ?? 200);
  const stop = before ? log.filter((e) => e.seq < Number(before)).length : log.length;
  const start = Math.max(0, stop - limit);
  return { events: log.slice(start, stop), facts: start > 0 ? facts(log.slice(0, start), log[start].seq) : null };
});

function facts(earlier: SessionEvent[], before: number): EarlierFacts {
  const top = earlier.filter((e) => e.body.kind !== "subagent_event");
  const lastAsk = [...top].reverse().find((e) => e.body.kind === "ask_user") ?? null;
  const askId = lastAsk?.body.kind === "ask_user" ? lastAsk.body.request_id : null;
  const answered = askId ? (world.events[earlier[0]?.session_id] ?? []).find((e) => e.body.kind === "ask_answered" && e.body.request_id === askId) : undefined;
  const turns = top.filter((e) => e.body.kind === "turn_end");
  return {
    before,
    first_user: top.find((e) => e.body.kind === "user_message") ?? null,
    last_assistant: [...top].reverse().find((e) => e.body.kind === "assistant_message") ?? null,
    last_ask: lastAsk,
    last_ask_answers: answered?.body.kind === "ask_answered" ? answered.body.answers : null,
    turns: turns.length,
    turns_duration_ms: turns.every((e) => e.body.kind === "turn_end" && e.body.duration_ms != null) ? turns.reduce((n, e) => n + (e.body.kind === "turn_end" ? e.body.duration_ms ?? 0 : 0), 0) : null,
    subagents: top.filter((e) => e.body.kind === "subagent"),
  };
}

/** `launchpad trace`: the stored events as plain text. */
route("GET", "/api/sessions/:id/trace", (r) => {
  const through = Number(r.query.get("through") ?? Infinity);
  const lines = (world.events[r.params.id] ?? []).filter((e) => e.seq <= through).map((e) => {
    const b = e.body;
    switch (b.kind) {
      case "user_message": return `## User\n${b.text}`;
      case "assistant_message": return `## Assistant\n${b.text}`;
      case "reasoning": return `(thinking) ${b.text}`;
      case "tool_call": return `$ ${b.name}: ${b.summary}`;
      case "tool_result": return b.output;
      case "file_change": return `[${b.change}] ${b.path}`;
      case "notice": return `(notice) ${b.text}`;
      default: return null;
    }
  }).filter(Boolean);
  return new Response(lines.join("\n\n"), { headers: { "Content-Type": "text/plain; charset=utf-8" } });
});

// ---------------------------------------------------------------- Git (Changes tab)

const gitFixture = (id: string) => {
  const s = found(id);
  const g = world.sessionGit[id];
  if (!g) {
    if (!s.project_id || s.project_id === "demo-tmp-csv" || s.cwd.includes("/assistant/")) throw new DemoError(400, "not a git repository");
  }
  return g ?? null;
};

route("GET", "/api/sessions/:id/git", (r): SessionGitStatus => {
  const g = gitFixture(r.params.id);
  if (!g) return { branch: "main", files: [], unpushed_commits: 0, main_changes: null, main_files: [] };
  const all = g.files.map((f) => ({ ...f, status: g.committed ? "M " : f.status }));
  const counts = Object.values(g.patches).join("\n").split("\n");
  return {
    branch: g.branch, files: g.committed || g.merged_into ? [] : all, unpushed_commits: g.commits.length,
    main_changes: g.merged_into || !g.files.length ? null : { files: g.files.length, additions: counts.filter((l) => l.startsWith("+") && !l.startsWith("+++")).length, deletions: counts.filter((l) => l.startsWith("-") && !l.startsWith("---")).length },
    main_files: g.merged_into ? [] : g.files.map((f) => ({ ...f, status: f.status.trim() === "??" ? "A" : f.status.trim() })),
  };
});
route("GET", "/api/sessions/:id/git-commits", (r): SessionGitHistory | null => {
  const g = gitFixture(r.params.id);
  if (!g || !g.commits.length) return null;
  const commit = r.query.get("commit");
  const head = g.commits.at(-1)!.id;
  if (commit && !g.commits.some((c) => c.id === commit)) throw new DemoError(400, "unknown commit");
  return { commits: [...g.commits].reverse(), files: g.files.map((f) => ({ ...f, status: f.status.trim() === "??" ? "A" : f.status.trim() })), merged_into: g.merged_into, base: "0a1b2c3d4e5f60718293a4b5c6d7e8f901234567", head: commit ?? head };
});
route("GET", "/api/sessions/:id/git-diff", (r): SessionGitDiff => {
  const g = gitFixture(r.params.id);
  const path = r.query.get("path") ?? "";
  const patch = g?.patches[path];
  if (!patch) throw new DemoError(400, "file is no longer changed; refresh status");
  return { staged: "", unstaged: patch, truncated: false };
});

// ---------------------------------------------------------------- background commands

route("GET", "/api/sessions/:id/task-output", (r) => {
  const text = world.taskOutputs[r.params.id]?.[r.query.get("task_id") ?? ""] ?? null;
  return { text };
});
route("GET", "/api/sessions/:id/processes", (r): SessionProcess[] => {
  const s = found(r.params.id);
  return s.harness === "codex" && s.status === "background" ? [{ thread_id: "t1", process_id: "p1", command: "cargo watch -x test", cwd: s.cwd, output: "[Running 'cargo test']\ntest result: ok. 58 passed" }] : [];
});
route("POST", "/api/sessions/:id/processes/stop", (r) => { found(r.params.id); return { ok: true }; });

// ---------------------------------------------------------------- read state and titles

route("POST", "/api/sessions/:id/seen", (r) => {
  const s = found(r.params.id);
  if (s.seen_at < s.status_since) { updateSession(s.id, { seen_at: Math.max(s.status_since, clock.now()) }); refreshCounts(); }
  return session(s.id);
});
route("POST", "/api/sessions/:id/unread", (r) => {
  const s = found(r.params.id);
  if (["waiting", "error", "interrupted"].includes(s.status)) updateSession(s.id, { seen_at: s.status_since - 1 });
  refreshCounts();
  return session(s.id);
});
route("POST", "/api/sessions/:id/rename", (r) => {
  const title = String(r.body?.title ?? "").trim();
  if (!title) throw new DemoError(400, "the title is empty");
  updateSession(found(r.params.id).id, { title });
  return session(r.params.id);
});

// ---------------------------------------------------------------- run control

/** Stops a scenario and records the end of the turn like an owner Stop. */
function stopTurn(s: Session) {
  if (stopRun(s.id) || ["working", "starting"].includes(s.status)) {
    appendEvent(s.id, { kind: "turn_end", duration_ms: null, interrupted: true, error: null });
  }
}

route("POST", "/api/sessions/:id/interrupt", (r) => {
  const s = found(r.params.id);
  if (s.quota_pause) {
    stopRun(s.id);
    delete world.dormant[s.id];
    updateSession(s.id, { quota_pause: null });
    setStatus(s.id, "waiting");
    appendEvent(s.id, { kind: "notice", text: "Automatic quota continuation cancelled by you. Held input will not be sent." });
    return { ok: true };
  }
  if (s.status === "background") {
    for (const e of world.events[s.id] ?? []) if (e.body.kind === "background_task" && e.body.status === "running") appendEvent(s.id, { ...e.body, status: "stopped" });
    setStatus(s.id, "stopped");
    return { ok: true };
  }
  if (!s.live) throw new DemoError(400, "the session has no running process");
  stopTurn(s);
  delete world.dormant[s.id];
  updateSession(s.id, { open_question: false });
  setStatus(s.id, "stopped");
  return { ok: true };
});

route("POST", "/api/sessions/:id/close", (r) => {
  const s = found(r.params.id);
  stopTurn(s);
  delete world.dormant[s.id];
  const done = s.status === "waiting";
  updateSession(s.id, { quota_pause: null });
  setStatus(s.id, "closed", { live: false, open_question: false, ended_at: clock.now(), archived_done: done });
  refreshCounts();
  return { ok: true };
});

route("POST", "/api/sessions/:id/resume", (r) => {
  const s = found(r.params.id);
  if (s.status === "in_terminal") {
    for (const t of world.terminals.filter((t) => t.session_id === s.id && t.kind === "tui")) removeDemoTerminal(t.id);
    appendEvent(s.id, { kind: "notice", text: "Re-attached: the session continues in Launchpad." });
    setStatus(s.id, "waiting", { live: true });
    return { ok: true };
  }
  if (s.status === "closed") { setStatus(s.id, "waiting", { live: true, archived_done: false }); return { ok: true }; }
  if (s.status === "quota_paused") { continueQuota(s.id); return { ok: true }; }
  if (isPlaying(s.id)) return { ok: true };
  const unfinished = ["stopped", "interrupted", "error"].includes(s.status);
  if (unfinished || r.body?.continue_turn) {
    appendEvent(s.id, { kind: "user_message", text: "Continue the unfinished turn.", client_id: `resume-${s.last_seq}`, queued: false, images: [] });
    playScenario(s.id, "follow-up");
  } else setStatus(s.id, "waiting", { live: true });
  return { ok: true };
});
route("POST", "/api/sessions/:id/resume-paid", (r) => {
  const s = found(r.params.id);
  updateSession(s.id, { launch_spec: { ...s.launch_spec, allow_paid_usage: true } });
  appendEvent(s.id, { kind: "notice", text: "Paid usage allowed for this session. Continuing now." });
  continueQuota(s.id);
  return { ok: true };
});
route("POST", "/api/sessions/:id/resume-subscription", (r) => {
  const s = found(r.params.id);
  appendEvent(s.id, { kind: "notice", text: "Retrying with the subscription only." });
  continueQuota(s.id);
  return { ok: true };
});

/** A message: the next scenario plays (line notes, a reply, or the assistant's answer). */
export function sendMessage(s: Session, text: string, clientId: string, files: { name: string }[] = []) {
  if (s.status === "in_terminal") throw new DemoError(400, "the session is open in a terminal: re-attach it first");
  if (s.quota_pause) throw new DemoError(400, "This session is quota paused. Use a Resume button or Stop first.");
  const full = files.length ? `${text}\n\nAttached files:\n${files.map((f) => `- ${f.name}`).join("\n")}` : text;
  appendEvent(s.id, { kind: "user_message", text: full, client_id: clientId, queued: false, images: [] });
  if (isPlaying(s.id)) return;
  if (s.status === "closed") updateSession(s.id, { archived_done: false });
  playScenario(s.id, pickReply(text, isAssistant(s)));
}

route("POST", "/api/sessions/:id/input", (r) => {
  const s = found(r.params.id);
  let text = String(r.body?.text ?? "");
  const context = r.body?.context as MessageContext | undefined;
  const lines: string[] = [];
  for (const c of context?.context_projects ?? []) {
    const project = world.projects.find((p) => p.id === c.project_id);
    if (!project) throw new DemoError(400, "context project no longer exists");
    lines.push(`- Reference project: ${project.path}. Read it, but do not modify it.`);
  }
  for (const g of context?.github_refs ?? []) lines.push(`- ${g.title ?? g.repo}: ${g.url}`);
  for (const t of context?.agent_traces ?? []) {
    const source = found(t.session_id);
    lines.push(`- Reference agent trace: ${source.title}, through event ${t.through_seq}.`);
  }
  if (lines.length) text = [text.trim(), `Context:\n${lines.join("\n")}`].filter(Boolean).join("\n\n");
  const files = (r.body?.files ?? []) as { name: string }[];
  if (!text.trim() && !files.length) throw new DemoError(400, "the message is empty");
  if (s.open_question) throw new DemoError(400, "Answer the open question first.");
  sendMessage(s, text, r.headers.get("Idempotency-Key") ?? `c-${s.last_seq + 1}`, files);
  return { ok: true };
});

route("POST", "/api/sessions/:id/answer", (r) => {
  const s = found(r.params.id);
  const requestId = String(r.body?.request_id ?? "");
  const answers = (r.body?.answers ?? []) as string[];
  if (!s.live) throw new DemoError(400, "the session has no running process");
  if (answerRun(s.id, requestId, answers)) return { ok: true };
  // A question outside a scenario: record the answer and reply.
  appendEvent(s.id, { kind: "ask_answered", request_id: requestId, answers });
  updateSession(s.id, { open_question: false });
  playScenario(s.id, "follow-up");
  return { ok: true };
});

// ---------------------------------------------------------------- run settings

route("POST", "/api/sessions/:id/paid-usage", (r) => {
  const s = found(r.params.id);
  const allow = !!r.body?.allow;
  updateSession(s.id, { launch_spec: { ...s.launch_spec, allow_paid_usage: allow } });
  appendEvent(s.id, { kind: "notice", text: allow ? "Paid usage allowed for this session." : "This session uses the subscription only." });
  return session(s.id);
});
route("POST", "/api/sessions/:id/budget", (r) => {
  const s = found(r.params.id);
  const usd: number | null = r.body?.budget_usd ?? null;
  updateSession(s.id, { launch_spec: { ...s.launch_spec, budget_usd: usd } });
  appendEvent(s.id, { kind: "notice", text: usd == null ? "Cost limit removed." : `Cost limit set to $${usd.toFixed(2)}.` });
  return session(s.id);
});
route("POST", "/api/sessions/:id/permissions", (r) => {
  const s = found(r.params.id);
  const mode = r.body?.mode as PermissionMode;
  if (!["yolo", "approvals", "auto"].includes(mode)) throw new DemoError(400, "unknown permission mode");
  updateSession(s.id, { launch_spec: { ...s.launch_spec, permission_mode: mode } });
  updateRunOptions(s.id, { permission: mode });
  appendEvent(s.id, { kind: "notice", text: `Permission mode set to ${{ yolo: "YOLO", approvals: "Ask for approval", auto: "Auto approval" }[mode]}.` });
  return session(s.id);
});
route("POST", "/api/sessions/:id/model", (r) => {
  const s = found(r.params.id);
  const model: string | null = r.body?.model || null;
  const effort: string | null = r.body?.effort || null;
  updateSession(s.id, { model: model ?? s.model, effort });
  appendEvent(s.id, { kind: "notice", text: `Model set to ${model ?? "the default"}${effort ? ` (${effort} effort)` : ""}. It applies from the next turn.` });
  if (isAssistant(s) && model) { world.config.assistant = { ...world.config.assistant, model, effort }; publish("config", "changed", null); }
  return session(s.id);
});

// ---------------------------------------------------------------- terminal handoff

route("POST", "/api/sessions/:id/terminal", (r) => {
  const s = found(r.params.id);
  if (isPlaying(s.id) || ["working", "starting"].includes(s.status)) throw new DemoError(400, "Stop the running turn first, then continue in the terminal.");
  const t = openDemoTerminal({ kind: "tui", cwd: s.cwd, session_id: s.id });
  setStatus(s.id, "in_terminal", { live: false });
  appendEvent(s.id, { kind: "notice", text: `Continued in the ${s.harness === "codex" ? "Codex" : s.harness === "opencode" ? "OpenCode" : "Claude Code"} TUI in a terminal. Re-attach to continue here.` });
  return t;
});

// Local PR notes sent to their agent (Review → Local PR): the agent addresses them.
localPrHooks.feedback = (p, text) => {
  const s = p.session_id ? session(p.session_id) : undefined;
  if (s && !isPlaying(s.id) && s.status !== "in_terminal" && !s.quota_pause) sendMessage(s, text, `notes-${s.last_seq + 1}`);
};
