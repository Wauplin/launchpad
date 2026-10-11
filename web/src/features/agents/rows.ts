// Turns a session's event log into display rows.

import type { ImageAttachment } from "@/gen/ImageAttachment";
import type { PlanItem } from "@/gen/PlanItem";
import type { Question } from "@/gen/Question";
import type { SessionLog } from "@/lib/sessionStore";

export type Call = {
  id: string;
  revision: number;
  name: string;
  summary: string;
  input: unknown;
  result?: { output: string; is_error: boolean; exit_code: number | null; duration_ms: number | null };
  live?: string;
  ts: number;
};

export type Row = { key: string; v: string } & (
  | { t: "user"; text: string; queued: boolean; images: ImageAttachment[] }
  | { t: "assistant"; text: string; streaming: boolean }
  | { t: "reasoning"; text: string }
  | { t: "tools"; calls: Call[] }
  | { t: "edit"; path: string; change: string; diff: string }
  | { t: "plan"; items: PlanItem[] }
  | { t: "ask"; request_id: string; questions: Question[]; answers: string[] | null }
  | { t: "bg"; task_id: string; description: string; status: string; output_file: string | null; ts: number }
  | { t: "turn"; ts: number; interrupted: boolean; error: string | null; duration_ms: number | null }
  | { t: "notice"; text: string }
  | { t: "error"; text: string }
);

export function buildRows(log: SessionLog): Row[] {
  const rows: Row[] = [];
  const delivered = new Set<string>();
  const calls = new Map<string, Call>();
  const asks = new Map<string, Extract<Row, { t: "ask" }>>();
  const tasks = new Map<string, Extract<Row, { t: "bg" }>>();
  let lastPlan = -1;
  for (const e of log.events) if (e.body.kind === "message_delivered") delivered.add(e.body.client_id);
  log.events.forEach((e, i) => {
    if (e.body.kind === "plan") lastPlan = i;
  });

  log.events.forEach((e, i) => {
    const b = e.body;
    const key = `e${e.seq}`;
    switch (b.kind) {
      case "user_message":
        rows.push({ key, v: "", t: "user", text: b.text, images: b.images ?? [], queued: b.queued && !delivered.has(b.client_id) });
        break;
      case "assistant_message":
        rows.push({ key, v: String(b.text.length), t: "assistant", text: b.text, streaming: false });
        break;
      case "reasoning":
        rows.push({ key, v: "", t: "reasoning", text: b.text });
        break;
      case "tool_call": {
        const existing = calls.get(b.item_id);
        if (existing) {
          existing.name = b.name; existing.summary = b.summary; existing.input = b.input; existing.revision = e.seq;
          break;
        }
        const call: Call = { id: b.item_id, revision: e.seq, name: b.name, summary: b.summary, input: b.input, ts: e.ts };
        calls.set(b.item_id, call);
        const last = rows[rows.length - 1];
        if (last?.t === "tools") last.calls.push(call);
        else rows.push({ key, v: "", t: "tools", calls: [call] });
        break;
      }
      case "tool_result": {
        const c = calls.get(b.item_id);
        if (c) c.result = { output: b.output, is_error: b.is_error, exit_code: b.exit_code, duration_ms: b.duration_ms ?? e.ts - c.ts };
        break;
      }
      case "file_change":
        rows.push({ key, v: "", t: "edit", path: b.path, change: b.change, diff: b.diff });
        break;
      case "plan":
        if (i === lastPlan) rows.push({ key, v: "", t: "plan", items: b.items });
        break;
      case "ask_user": {
        const r: Extract<Row, { t: "ask" }> = { key, v: "", t: "ask", request_id: b.request_id, questions: b.questions, answers: null };
        asks.set(b.request_id, r);
        rows.push(r);
        break;
      }
      case "ask_answered": {
        const r = asks.get(b.request_id);
        if (r) r.answers = b.answers;
        break;
      }
      case "background_task": {
        const r = tasks.get(b.task_id);
        if (r) {
          r.status = b.status;
          r.output_file = b.output_file ?? r.output_file;
          if (b.description && r.description === "") r.description = b.description;
        } else {
          const row: Extract<Row, { t: "bg" }> = { key, v: "", t: "bg", task_id: b.task_id, description: b.description, status: b.status, output_file: b.output_file, ts: e.ts };
          tasks.set(b.task_id, row);
          rows.push(row);
        }
        break;
      }
      case "turn_end":
        rows.push({ key, v: "", t: "turn", ts: e.ts, interrupted: b.interrupted, error: b.error, duration_ms: b.duration_ms });
        break;
      case "notice":
        rows.push({ key, v: "", t: "notice", text: b.text });
        break;
      case "error":
        rows.push({ key, v: "", t: "error", text: b.message });
        break;
      default:
        break;
    }
  });

  for (const [id, text] of Object.entries(log.outputs)) {
    const c = calls.get(id);
    if (id.includes("\0")) continue;
    if (c && !c.result) c.live = text;
  }
  for (const [id, text] of Object.entries(log.deltas)) {
    if (id.includes("\0")) continue;
    if (text.trim()) rows.push({ key: `d${id}`, v: String(text.length), t: "assistant", text, streaming: true });
  }
  // Versions, so memoized rows only render again when they change.
  for (const r of rows) {
    if (r.t === "tools") r.v = r.calls.map((c) => `${c.id}:${c.revision}${c.result ? "r" : ""}${c.live?.length ?? ""}`).join(",");
    else if (r.t === "ask") r.v = r.answers ? "a" : "";
    else if (r.t === "bg") r.v = `${r.status}${r.output_file ?? ""}`;
    else if (r.t === "user") r.v = r.queued ? "q" : "";
  }
  return rows;
}

const KINDS: [RegExp, string, string][] = [
  [/^(Bash|Shell)$/, "command", "commands"],
  [/^Read$/, "file read", "files read"],
  [/^(Edit|Write|MultiEdit|NotebookEdit)$/, "edit", "edits"],
  [/^(Grep|Glob|ToolSearch)$/, "search", "searches"],
  [/^(WebFetch|WebSearch|Web search)$/, "web request", "web requests"],
  [/^(Task|Agent)$/, "subagent", "subagents"],
];

/** "Ran 3 commands, 5 files read" for a collapsed group. */
export function groupSummary(calls: Call[]): string {
  const counts = new Map<string, [number, string, string]>();
  for (const c of calls) {
    const k = KINDS.find(([re]) => re.test(c.name));
    const [one, many] = k ? [k[1], k[2]] : ["tool call", "tool calls"];
    const cur = counts.get(one) ?? [0, one, many];
    cur[0]++;
    counts.set(one, cur);
  }
  const parts = [...counts.values()].map(([n, one, many]) => `${n} ${n === 1 ? one : many}`);
  const text = parts.join(", ");
  return text.charAt(0).toUpperCase() + text.slice(1);
}
