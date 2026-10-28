// Agents part of the demo world: sessions with their stored events, scheduled launches, crons, Ask proposals,
// terminals and task worktrees.

import type { EventBody } from "@/gen/EventBody";
import type { Session } from "@/gen/Session";
import type { SessionEvent } from "@/gen/SessionEvent";
import { clock } from "../daemon";
import { defaultSpec } from "../store";
import { P, ROOT, WS } from "./ids";

type Build = Partial<Session> & { id: string; title: string; project: string; minutesAgo: number; events?: EventBody[] };

/** A demo session whose stored events are `events`, spaced out over the last `minutesAgo` minutes. */
export function demoSession(b: Build, log: Record<string, SessionEvent[]>): Session {
  const started = clock.ago(b.minutesAgo);
  const events = b.events ?? [];
  log[b.id] = events.map((body, i) => ({ session_id: b.id, seq: i + 1, ts: started + Math.round(((i + 1) / (events.length + 1)) * b.minutesAgo * 60_000 * 0.9), body }));
  const last = [...events].reverse().find((e) => e.kind === "assistant_message");
  const status = b.status ?? "waiting";
  const s: Session = {
    harness: "claude", harness_session_id: null, project_id: b.project, cwd: `${ROOT}/${b.project.replace(/^demo-/, "")}`, status,
    archived_done: false, status_since: clock.ago(Math.max(1, b.minutesAgo / 4)), live: ["starting", "working", "background", "waiting"].includes(status),
    open_question: false, model: "sonnet", effort: null, prompt: b.title, usage: { context_tokens: 42_000, context_window: 200_000, input_tokens: 180_000, cached_tokens: 120_000, output_tokens: 9_000, cost_usd: null, cost_unavailable: false },
    started_at: started, ended_at: null, last_seq: events.length, launch_spec: defaultSpec({ target: { kind: "project", project_id: b.project }, text: b.title, workspace_id: WS.acme }),
    quota_pause: null, seen_at: clock.ago(b.minutesAgo / 2), last_message: last && last.kind === "assistant_message" ? last.text.split("\n")[0] : null,
    ...b,
  } as Session;
  delete (s as Partial<Build>).minutesAgo;
  delete (s as Partial<Build>).events;
  delete (s as Partial<Build>).project;
  return s;
}

export function agentsWorld() {
  const events: Record<string, SessionEvent[]> = {};
  const sessions: Session[] = [
    demoSession({
      id: "demo-s-checkout", title: "Fix the flaky checkout test", project: P.storefront, minutesAgo: 25, status: "waiting", seen_at: 0,
      events: [
        { kind: "user_message", text: "Fix the flaky checkout test", client_id: "c1", queued: false, images: [] },
        { kind: "reasoning", item_id: "r1", text: "The test waits for a fixed timeout; it should wait for the order request instead." },
        { kind: "tool_call", item_id: "t1", name: "Bash", summary: "pnpm test checkout", input: { command: "pnpm test checkout" } },
        { kind: "tool_result", item_id: "t1", output: "1 failed, 41 passed", is_error: true, exit_code: 1, duration_ms: 5400 },
        { kind: "assistant_message", item_id: "a1", text: "The checkout test now waits for the order request instead of a fixed timeout. All 42 tests pass." },
        { kind: "turn_end", duration_ms: 64_000, interrupted: false, error: null },
        { kind: "status", status: "waiting" },
      ],
    }, events),
    demoSession({
      id: "demo-s-invoices", title: "Rate-limit /invoices", project: P.billing, minutesAgo: 4, status: "working", harness: "codex", model: "gpt-5",
      events: [
        { kind: "user_message", text: "Rate-limit /invoices to 10 requests per minute", client_id: "c2", queued: false, images: [] },
        { kind: "reasoning", item_id: "r2", text: "Looking at the router middleware." },
      ],
    }, events),
    demoSession({
      id: "demo-s-docs", title: "Document the refund flow", project: P.docs, minutesAgo: 180, status: "waiting", harness: "opencode", model: "qwen3-coder",
      events: [
        { kind: "user_message", text: "Document the refund flow", client_id: "c3", queued: false, images: [] },
        { kind: "assistant_message", item_id: "a3", text: "Added `refunds.md` with the flow, the states and two examples." },
        { kind: "status", status: "waiting" },
      ],
    }, events),
  ];
  return { sessions, events };
}
