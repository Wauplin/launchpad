// Mutations of the demo world that the UI must hear about: each one updates the world and publishes the same bus
// event the real daemon would, so the unchanged UI (TanStack Query cache, session logs) follows along.

import type { EventBody } from "@/gen/EventBody";
import type { LaunchSpec } from "@/gen/LaunchSpec";
import type { Session } from "@/gen/Session";
import type { SessionEvent } from "@/gen/SessionEvent";
import type { SessionStatus } from "@/gen/SessionStatus";
import { clock, publish } from "./daemon";
import { world } from "./world";

/** `Session::light()`: the list and bus form, with the prompt cut at 4,000 bytes. */
export const light = (s: Session): Session => ({ ...s, prompt: s.prompt.slice(0, 4000) });

export function session(id: string): Session | undefined {
  return world.sessions.find((s) => s.id === id);
}

/** Publishes the session's current state (`sessions/updated`). */
export function publishSession(s: Session) {
  publish("sessions", "updated", light(s));
}

/** Updates a session and tells every window. A status change also moves `status_since`. */
export function updateSession(id: string, patch: Partial<Session>) {
  const s = session(id);
  if (!s) return;
  if (patch.status && patch.status !== s.status) patch = { status_since: clock.now(), ...patch };
  Object.assign(s, patch);
  publishSession(s);
  refreshCounts();
}

/** Appends a stored event (or a live-only one with `live`), like `Sink::emit`. */
export function appendEvent(id: string, body: EventBody, live = false): SessionEvent {
  const s = session(id);
  const log = (world.events[id] ??= []);
  const seq = live ? 0 : (log.at(-1)?.seq ?? 0) + 1;
  const ev: SessionEvent = { session_id: id, seq, ts: clock.now(), body };
  if (!live) {
    log.push(ev);
    if (s) {
      s.last_seq = seq;
      if (body.kind === "assistant_message") s.last_message = body.text.split("\n").find((l) => l.trim())?.slice(0, 400) ?? null;
    }
  }
  publish(`session/${id}`, "event", ev);
  return ev;
}

/** Sets a session status: a status event in its log and the updated session. */
export function setStatus(id: string, status: SessionStatus, patch: Partial<Session> = {}) {
  appendEvent(id, { kind: "status", status });
  updateSession(id, { status, live: !["closed", "stopped", "interrupted", "error", "in_terminal"].includes(status), ...patch });
}

/** Recomputes `Status.sessions` with the shared predicates and publishes the status. */
export function refreshCounts() {
  const normal = world.sessions.filter((s) => s.launch_spec.target.kind !== "assistant");
  const running = normal.filter((s) => ["starting", "working", "background"].includes(s.status)).length;
  const quota_paused = normal.filter((s) => s.status === "quota_paused").length;
  const done = normal.filter((s) => (s.status === "waiting" && (s.open_question || s.seen_at < s.status_since)) || ((s.status === "error" || s.status === "interrupted") && s.seen_at < s.status_since)).length;
  world.status = { ...world.status, sessions: { running, done, quota_paused } };
  publish("status", "updated", world.status);
}

export const defaultSpec = (patch: Partial<LaunchSpec> = {}): LaunchSpec => ({
  permission_mode: "yolo", allow_paid_usage: false, harness: "claude", profile: "claude-max", budget_usd: null,
  target: { kind: "none" }, worktree: null, text: "", attachments: [], presets: [], context_projects: [], github_refs: [], files: [],
  agent_traces: [], model: null, effort: null, workspace_id: null, ...patch,
});

let counter = 0;
/** A fresh demo ID with a readable prefix. */
export const demoId = (prefix: string) => `${prefix}-${(++counter).toString(36)}${Math.floor(clock.now() % 1e6).toString(36)}`;
