// The page chat's rows: one bot session's events drawn like the bot chat in Launchpad, smaller. Per turn the message,
// the bot's last answer and one steps line ("6 steps · 40 s"); a turn that still works shows its current step. Pure.

import type { EventBody } from "@/gen/EventBody";
import type { SessionEvent } from "@/gen/SessionEvent";

export type ChatRow =
  | { t: "user"; key: string; text: string; from: string | null; queued: boolean }
  | { t: "answer"; key: string; text: string; streaming: boolean }
  | { t: "steps"; key: string; count: number; durationMs: number | null }
  | { t: "ask"; key: string; question: string }
  | { t: "ended"; key: string; text: string }
  | { t: "working"; key: string; step: string | null; since: number };

const line = (s: string, max = 90) => {
  const l = s.trim().split("\n")[0] ?? "";
  return l.length > max ? `${l.slice(0, max - 1)}…` : l;
};

function stepOf(b: EventBody): string | null {
  switch (b.kind) {
    case "tool_call": return `${b.name} ${line(b.summary, 80)}`.trim();
    case "reasoning": return "Thinking";
    case "file_change": return `${b.change === "add" ? "Created" : b.change === "delete" ? "Deleted" : "Edited"} ${b.path}`;
    case "plan": return "Updated the plan";
    case "background_task": return `Background: ${line(b.description || b.task_id, 80)}`;
    default: return null;
  }
}

/** Home's header of a message that is not the owner's starts with this; the sender's name follows. */
function fromOf(e: Extract<EventBody, { kind: "user_message" }>) {
  return e.from ? e.from.name || e.from.kind : null;
}

/** The rows of one session's events (oldest first); `deltas` holds the streamed text of answers not final yet. */
export function chatRows(events: SessionEvent[], running: boolean, deltas: Record<string, string>): ChatRow[] {
  const out: ChatRow[] = [];
  type Turn = { key: string; start: number; steps: string[]; answer: { key: string; text: string } | null; asks: ChatRow[]; end: Extract<EventBody, { kind: "turn_end" }> | null };
  let turn: Turn | null = null;
  const flush = (last: boolean) => {
    if (!turn) return;
    const t = turn;
    const working = last && !t.end && running;
    const streamed = Object.entries(deltas).filter(([k]) => !k.includes("\0")).at(-1);
    if (t.answer) out.push({ t: "answer", key: t.answer.key, text: t.answer.text, streaming: false });
    else if (working && streamed) out.push({ t: "answer", key: `d:${streamed[0]}`, text: streamed[1], streaming: true });
    if (t.steps.length && !working) out.push({ t: "steps", key: `s:${t.key}`, count: t.steps.length, durationMs: t.end?.duration_ms ?? null });
    out.push(...t.asks);
    if (t.end?.error) out.push({ t: "ended", key: `e:${t.key}`, text: line(t.end.error, 200) });
    else if (t.end?.interrupted) out.push({ t: "ended", key: `e:${t.key}`, text: "Stopped" });
    if (working) out.push({ t: "working", key: `w:${t.key}`, step: t.steps.at(-1) ?? null, since: t.start });
  };
  const open = (key: string, ts: number): Turn => ({ key, start: ts, steps: [], answer: null, asks: [], end: null });
  for (const ev of events) {
    const b = ev.body;
    const key = `e${ev.seq}`;
    if (b.kind === "user_message") {
      flush(false);
      out.push({ t: "user", key, text: b.text, from: fromOf(b), queued: b.queued });
      turn = open(key, ev.ts);
      continue;
    }
    turn ??= open(key, ev.ts);
    if (b.kind === "assistant_message") {
      // An earlier answer of the same turn becomes a step: only the last one shows.
      if (turn.answer) turn.steps.push(line(turn.answer.text));
      turn.answer = { key, text: b.text };
    } else if (b.kind === "ask_user") {
      turn.asks.push({ t: "ask", key, question: b.questions.map((q) => q.question).join(" · ") || "The bot asks you something" });
    } else if (b.kind === "ask_answered") {
      turn.asks = [];
    } else if (b.kind === "turn_end") {
      turn.end = b;
    } else {
      const step = stepOf(b);
      if (step) {
        // A step after the answer means the answer was not the last word.
        if (turn.answer) { turn.steps.push(line(turn.answer.text)); turn.answer = null; }
        turn.steps.push(step);
      }
    }
    if (b.kind === "turn_end") { flush(false); turn = null; }
  }
  flush(true);
  if (running && (!out.length || out.at(-1)!.t !== "working") && !turn) out.push({ t: "working", key: "w", step: null, since: Date.now() });
  return out;
}

/** "40 s", "3 min", "1 h 5 min". */
export function duration(ms: number) {
  const s = Math.max(0, Math.round(ms / 1000));
  if (s < 60) return `${s} s`;
  const m = Math.round(s / 60);
  return m < 60 ? `${m} min` : `${Math.floor(m / 60)} h ${m % 60} min`;
}
