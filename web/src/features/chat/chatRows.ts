// The chat of a bot's thread: a second way to draw the same session events (no storage of its own). Per turn it
// keeps the message (the owner's, or another bot's or agent's under its sender), the bot's last answer, one steps line for the work behind it, its questions, and the
// cards written during the turn; a routine run that ends with `no_update` is one quiet line. Each session of the
// thread starts with a divider. Pure, so it is tested (tests/chatRows.test.mjs).

import type { ImageAttachment } from "@/gen/ImageAttachment";
import type { Row } from "../agents/rows.ts";
import type { TraceRow } from "../agents/traceRows.ts";
import type { MessageFrom } from "@/gen/MessageFrom";
import { isNoUpdate, routineMessage } from "../../lib/routines.ts";
import { fromAgent, withoutSender } from "../../lib/senders.ts";

type AskRow = Extract<Row, { t: "ask" }>;

/** One loaded session of the thread, with its display rows (`foldTrace(buildRows(log))`). */
export type ChatSession = {
  id: string;
  started_at: number;
  profile: string | null;
  /** The folder ID, null for a temporary folder. */
  folder: string | null;
  /** Why it started (`BotLaunch.reason`: profile, folder, model, effort, permissions, archived, new, first). */
  reason?: string | null;
  /** The owner's words for a new session they asked for (`BotLaunch.note`). */
  note?: string | null;
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
  /** `instructions`: the message carried the new BOT.md (Home adds it after BOT.md changed), left out of `text`. */
  | { t: "user"; text: string; images: ImageAttachment[]; queued: boolean; instructions: boolean }
  /** A message from another bot or agent (Home's header and fence left out of `text`): never the owner's bubble. */
  | { t: "from"; text: string; images: ImageAttachment[]; queued: boolean; instructions: boolean; from: MessageFrom }
  | { t: "routine"; title: string; cause: string }
  | { t: "quiet"; title: string; cause: string; at: number | null }
  | { t: "answer"; text: string; streaming: boolean }
  | { t: "steps"; sessionId: string; count: number; durationMs: number | null; steps: string[] }
  | { t: "ask"; sessionId: string; row: AskRow }
  | { t: "ended"; sessionId: string; interrupted: boolean; error: string | null }
  | { t: "card"; kind: ChatCard["kind"]; id: string; text: string | null }
  | { t: "changes"; sessionId: string }
  | { t: "working"; sessionId: string; step: string | null }
  /** A hook or Telegram message waiting for the bot's turn to end (after every other row). */
  | { t: "queued"; n: number; at: number; source: string; text: string }
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

/** Why a session of the thread started (`bots::rotation`), for its divider. */
export const ROTATION_REASONS: Record<string, string> = {
  profile: "the profile changed",
  folder: "the folder changed",
  model: "the model changed",
  effort: "the effort changed",
  permissions: "the permission mode changed",
  archived: "the previous session was archived",
  new: "you asked for a new session",
};

function why(prev: ChatSession | undefined, s: ChatSession): string | null {
  if (s.reason === "new" && s.note) return `you asked for a new session: “${s.note}”`;
  if (s.reason) return ROTATION_REASONS[s.reason] ?? null;
  // Sessions from before the reason was recorded: what differs from the previous one.
  if (!prev) return null;
  if (prev.profile !== s.profile) return ROTATION_REASONS.profile;
  if (prev.folder !== s.folder) return ROTATION_REASONS.folder;
  return null;
}

/** How Home's block with a changed BOT.md starts (`bots::changed_block`), and how it ends. */
const CHANGED = "Your instructions changed. The owner edited BOT.md";
const CHANGED_END = "</bot-instructions>";

/** A message without the changed-BOT.md block Home added after its typed text, and whether it had one. */
export function withoutInstructions(text: string): { text: string; instructions: boolean } {
  const at = text.indexOf(CHANGED);
  if (at < 0) return { text, instructions: false };
  const end = text.indexOf(CHANGED_END, at);
  const rest = end < 0 ? "" : text.slice(end + CHANGED_END.length);
  return { text: `${text.slice(0, at).trimEnd()}${rest.trim() ? `\n\n${rest.trim()}` : ""}`, instructions: true };
}

/** The chat items of the loaded sessions (oldest first), with their cards placed at their turn. */
export function chatRows(sessions: ChatSession[], cards: ChatCard[] = [], first = 0): ChatItem[] {
  const out: ChatItem[] = [];
  sessions.forEach((s, si) => {
    // Event keys (`e<seq>`) repeat across sessions: every item key names its session.
    const k = (key: string) => `${s.id}/${key}`;
    const reason = why(sessions[si - 1], s);
    out.push({ key: k("div"), v: reason ?? "", t: "divider", sessionId: s.id, index: first + si + 1, at: s.started_at, reason });

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

    // Cards go to the first turn that ended after they were written (an unfinished turn other than the last, cut by
    // the next message, takes none); later ones to the last turn.
    const mine = cards.filter((c) => c.session_id === s.id && !(s.partial && s.firstTs !== null && c.at < s.firstTs)).sort((a, b) => a.at - b.at);
    const placed = turns.map(() => [] as ChatCard[]);
    const after: ChatCard[] = [];
    for (const c of mine) {
      const i = turns.findIndex((t, ti) => t.end ? t.end.ts >= c.at : ti === turns.length - 1);
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
        out.push({ key: k(`q:${t.key}`), v: "", t: "quiet", title: routine.title, cause: routine.cause, at: t.end.ts });
      } else {
        if (routine) out.push({ key: k(`r:${t.key}`), v: t.user!.queued ? "q" : "", t: "routine", title: routine.title, cause: routine.cause });
        else if (t.user && fromAgent(t.user.from)) { const m = withoutInstructions(withoutSender(t.user.text)); out.push({ key: k(t.user.key), v: t.user.queued ? "q" : "", t: "from", text: m.text, images: t.user.images, queued: t.user.queued, instructions: m.instructions, from: t.user.from }); }
        else if (t.user) { const m = withoutInstructions(t.user.text); out.push({ key: k(t.user.key), v: t.user.queued ? "q" : "", t: "user", text: m.text, images: t.user.images, queued: t.user.queued, instructions: m.instructions }); }
        if (answer) out.push({ key: k(answer.key), v: `${answer.text.length}${answer.streaming ? "s" : ""}`, t: "answer", text: answer.text, streaming: answer.streaming });
        if (steps.length && !working) out.push({ key: k(`s:${t.key}`), v: `${steps.length}:${t.end?.duration_ms ?? ""}`, t: "steps", sessionId: s.id, count: steps.length, durationMs: t.end?.duration_ms ?? null, steps });
      }
      for (const r of t.body) if (r.t === "ask") out.push({ key: k(r.key), v: r.answers ? "a" : "", t: "ask", sessionId: s.id, row: r });
      for (const c of placed[ti]) out.push({ key: `c:${c.kind}:${c.id}`, v: c.v, t: "card", kind: c.kind, id: c.id, text: c.text ?? null });
      if (t.end && (t.end.error || t.end.interrupted)) out.push({ key: k(`e:${t.end.key}`), v: "", t: "ended", sessionId: s.id, interrupted: t.end.interrupted, error: t.end.error });
      if (working) out.push({ key: k("w"), v: steps.at(-1) ?? "", t: "working", sessionId: s.id, step: steps.at(-1) ?? null });
    });
    if (!turns.length) for (const c of after) out.push({ key: `c:${c.kind}:${c.id}`, v: c.v, t: "card", kind: c.kind, id: c.id, text: c.text ?? null });
    if (s.running && (!turns.length || turns[turns.length - 1].end)) out.push({ key: k("w"), v: "", t: "working", sessionId: s.id, step: null });
    if (s.changes) out.push({ key: k("ch"), v: "", t: "changes", sessionId: s.id });
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
