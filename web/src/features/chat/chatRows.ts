// The chat of a bot's thread: a second way to draw the same session events (no storage of its own). Per turn it
// keeps the owner's message, the bot's last answer, one steps line for the work behind it, its questions, and the
// cards written during the turn; a routine run that ends with `no_update` is one quiet line. Each session of the
// thread starts with a divider. Pure, so it is tested (tests/chatRows.test.mjs).

import type { ImageAttachment } from "@/gen/ImageAttachment";
import type { Row } from "../agents/rows.ts";
import type { TraceRow } from "../agents/traceRows.ts";
import { isNoUpdate, routineMessage } from "../../lib/routines.ts";

type AskRow = Extract<Row, { t: "ask" }>;

/** One loaded session of the thread, with its display rows (`foldTrace(buildRows(log))`). */
export type ChatSession = {
  id: string;
  started_at: number;
  profile: string | null;
  /** The folder ID, null for a temporary folder. */
  folder: string | null;
  /** Starting, working or background: its last turn without an end gets the working line. */
  running: boolean;
  rows: TraceRow[];
  /** Timestamp of the first loaded event; earlier events (and the cards written then) are not loaded yet. */
  firstTs: number | null;
  /** Its first events are not loaded yet. */
  partial: boolean;
  /** Show the Changes card (branch and Merge) after its last turn. */
  changes: boolean;
};

/** Something written during a session, placed at the turn that wrote it: a proposal, a ready click, a change. */
export type ChatCard = { kind: "proposal" | "ready" | "changed"; id: string; session_id: string; at: number; v: string; text?: string };

export type ChatItem = { key: string; v: string } & (
  | { t: "divider"; sessionId: string; index: number; at: number; reason: string | null }
  | { t: "user"; text: string; images: ImageAttachment[]; queued: boolean }
  | { t: "routine"; title: string; cause: string }
  | { t: "quiet"; title: string; cause: string; at: number | null }
  | { t: "answer"; text: string; streaming: boolean }
  | { t: "steps"; sessionId: string; count: number; durationMs: number | null; steps: string[] }
  | { t: "ask"; sessionId: string; row: AskRow }
  | { t: "ended"; sessionId: string; interrupted: boolean; error: string | null }
  | { t: "card"; kind: ChatCard["kind"]; id: string; text: string | null }
  | { t: "changes"; sessionId: string }
  | { t: "working"; sessionId: string; step: string | null }
);

type Turn = {
  key: string;
  user: Extract<TraceRow, { t: "user" }> | null;
  /** Rows of the turn other than the message and its end. */
  body: TraceRow[];
  end: Extract<Row, { t: "turn" }> | null;
};

const firstLine = (text: string, max = 120) => {
  const line = text.trim().split("\n")[0] ?? "";
  return line.length > max ? `${line.slice(0, max - 1)}…` : line;
};

/** One line per step of the work behind an answer, without output. */
function stepLines(rows: TraceRow[]): string[] {
  const out: string[] = [];
  for (const r of rows) {
    switch (r.t) {
      case "reasoning": out.push("Thinking"); break;
      case "tools": for (const c of r.calls) out.push(`${c.name} ${firstLine(c.summary, 100)}`.trim()); break;
      case "edit": out.push(`${r.change === "add" ? "Created" : r.change === "delete" ? "Deleted" : "Edited"} ${r.path}`); break;
      case "assistant": out.push(firstLine(r.text)); break;
      case "plan": out.push("Updated the plan"); break;
      case "bg": out.push(`Background: ${firstLine(r.description || r.task_id, 100)}`); break;
      case "error": out.push(`Error: ${firstLine(r.text, 100)}`); break;
      default: break;
    }
  }
  return out;
}

function why(prev: ChatSession | undefined, s: ChatSession): string | null {
  if (!prev) return null;
  if (prev.profile !== s.profile) return "the profile changed";
  if (prev.folder !== s.folder) return "the folder changed";
  return null;
}

/** The chat items of the loaded sessions (oldest first), with their cards placed at their turn. */
export function chatRows(sessions: ChatSession[], cards: ChatCard[] = [], first = 0): ChatItem[] {
  const out: ChatItem[] = [];
  sessions.forEach((s, si) => {
    out.push({ key: `div:${s.id}`, v: "", t: "divider", sessionId: s.id, index: first + si + 1, at: s.started_at, reason: why(sessions[si - 1], s) });

    // Turns: a message starts one, a turn end closes it.
    const turns: Turn[] = [];
    let cur: Turn | null = null;
    for (const r of s.rows) {
      if (r.t === "user") {
        if (cur) turns.push(cur);
        cur = { key: r.key, user: r, body: [], end: null };
      } else if (r.t === "turn") {
        (cur ??= { key: r.key, user: null, body: [], end: null }).end = r;
        turns.push(cur);
        cur = null;
      } else {
        (cur ??= { key: r.key, user: null, body: [], end: null }).body.push(r);
      }
    }
    if (cur) turns.push(cur);

    // Cards go to the first turn that ended after they were written; later ones to the last turn.
    const mine = cards.filter((c) => c.session_id === s.id && !(s.partial && s.firstTs !== null && c.at < s.firstTs)).sort((a, b) => a.at - b.at);
    const placed = turns.map(() => [] as ChatCard[]);
    const after: ChatCard[] = [];
    for (const c of mine) {
      const i = turns.findIndex((t) => !t.end || t.end.ts >= c.at);
      if (i >= 0) placed[i].push(c); else after.push(c);
    }
    if (turns.length) placed[turns.length - 1].push(...after);

    turns.forEach((t, ti) => {
      const last = ti === turns.length - 1;
      const working = last && !t.end && s.running;
      // The answer: the turn's last assistant message, while it works only when nothing came after it.
      let answerAt = -1;
      for (let i = t.body.length - 1; i >= 0; i--) if (t.body[i].t === "assistant") { answerAt = i; break; }
      if (working && answerAt >= 0 && t.body.slice(answerAt + 1).some((r) => r.t !== "notice" && r.t !== "notices")) answerAt = -1;
      const answer = answerAt >= 0 ? t.body[answerAt] as Extract<Row, { t: "assistant" }> : null;
      const steps = stepLines(t.body.filter((_, i) => i !== answerAt));
      const routine = t.user ? routineMessage(t.user.text) : null;

      if (routine && t.end && !t.end.error && !t.end.interrupted && answer && !answer.streaming && isNoUpdate(answer.text)) {
        out.push({ key: `q:${t.key}`, v: "", t: "quiet", title: routine.title, cause: routine.cause, at: t.end.ts });
      } else {
        if (routine) out.push({ key: `r:${t.key}`, v: t.user!.queued ? "q" : "", t: "routine", title: routine.title, cause: routine.cause });
        else if (t.user) out.push({ key: t.user.key, v: t.user.queued ? "q" : "", t: "user", text: t.user.text, images: t.user.images, queued: t.user.queued });
        if (answer) out.push({ key: answer.key, v: `${answer.text.length}${answer.streaming ? "s" : ""}`, t: "answer", text: answer.text, streaming: answer.streaming });
        if (steps.length && !working) out.push({ key: `s:${t.key}`, v: `${steps.length}:${t.end?.duration_ms ?? ""}`, t: "steps", sessionId: s.id, count: steps.length, durationMs: t.end?.duration_ms ?? null, steps });
      }
      for (const r of t.body) if (r.t === "ask") out.push({ key: r.key, v: r.answers ? "a" : "", t: "ask", sessionId: s.id, row: r });
      for (const c of placed[ti]) out.push({ key: `c:${c.kind}:${c.id}`, v: c.v, t: "card", kind: c.kind, id: c.id, text: c.text ?? null });
      if (t.end && (t.end.error || t.end.interrupted)) out.push({ key: `e:${t.end.key}`, v: "", t: "ended", sessionId: s.id, interrupted: t.end.interrupted, error: t.end.error });
      if (working) out.push({ key: `w:${s.id}`, v: steps.at(-1) ?? "", t: "working", sessionId: s.id, step: steps.at(-1) ?? null });
    });
    if (!turns.length) for (const c of after) out.push({ key: `c:${c.kind}:${c.id}`, v: c.v, t: "card", kind: c.kind, id: c.id, text: c.text ?? null });
    if (s.running && (!turns.length || turns[turns.length - 1].end)) out.push({ key: `w:${s.id}`, v: "", t: "working", sessionId: s.id, step: null });
    if (s.changes) out.push({ key: `ch:${s.id}`, v: "", t: "changes", sessionId: s.id });
  });
  return out;
}

/** "7 steps · 41 s" */
export function stepsLabel(count: number, ms: number | null): string {
  const steps = `${count} step${count === 1 ? "" : "s"}`;
  if (ms == null) return steps;
  const sec = Math.round(ms / 1000);
  return `${steps} · ${sec < 60 ? `${sec} s` : `${Math.floor(sec / 60)} min ${sec % 60} s`}`;
}
