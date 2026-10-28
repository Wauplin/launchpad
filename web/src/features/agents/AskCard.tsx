// An open question of the agent with its options. Answers go to the harness at once (single choice) or
// with Send answer (multiple choice). Shared by the trace and the phone Now page.

import { useState } from "react";
import { toastError } from "@/components/Toast";
import { api } from "@/lib/api";
import type { Row } from "./rows";

export function AskCard({ row, sessionId, live, hint = "Or type another answer below." }: { row: Pick<Extract<Row, { t: "ask" }>, "request_id" | "questions" | "answers">; sessionId: string; live: boolean; hint?: string }) {
  const [picked, setPicked] = useState<Record<number, string[]>>({});
  const [busy, setBusy] = useState(false);
  const answered = row.answers !== null;
  const send = async (answers: string[]) => {
    setBusy(true);
    try {
      await api.post(`/api/sessions/${sessionId}/answer`, { request_id: row.request_id, answers });
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(false);
    }
  };
  const choose = (qi: number, label: string) => {
    const q = row.questions[qi];
    const cur = picked[qi] ?? [];
    const next = q.multi_select ? (cur.includes(label) ? cur.filter((l) => l !== label) : [...cur, label]) : [label];
    const all = { ...picked, [qi]: next };
    setPicked(all);
    if (!q.multi_select && row.questions.every((_, i) => (all[i] ?? []).length > 0)) {
      void send(row.questions.map((_, i) => all[i].join(", ")));
    }
  };
  return (
    <div data-tour={row.questions.some((q) => q.permission) ? "session.approval" : "session.question"} className={`grid gap-2 rounded-lg border px-2.5 py-[9px] ${answered ? "border-line-2 bg-chrome" : "border-attn-line bg-attn-soft"}`}>
      {row.questions.map((q, qi) => (
        <div key={qi} className="grid gap-1.5">
          {q.permission && <div className="text-[13px] font-semibold">Approval required</div>}
          <div className={`min-w-0 whitespace-pre-wrap break-words text-[13px] ${q.permission ? "pointer-events-auto max-h-72 overflow-auto rounded border border-line bg-paper p-2 font-mono" : "font-[650]"}`}>{q.question}</div>
          {answered ? (
            <div className="text-[13px] text-ink-2">
              → <b className="text-ink">{row.answers![qi]}</b>
            </div>
          ) : (
            q.options.map((o, oi) => {
              const on = (picked[qi] ?? []).includes(o.label);
              return (
                <button
                  key={oi}
                  disabled={busy || !live}
                  onClick={() => choose(qi, o.label)}
                  className={`flex min-h-[44px] items-center gap-2 rounded-md border px-2 py-1.5 text-left text-[13px] hover:border-teal disabled:opacity-60 ${on ? "border-teal bg-teal-soft" : "border-line bg-paper"}`}
                >
                  <span className="font-mono text-[11px] text-ink-3">{oi + 1}</span>
                  <span>
                    <b className="font-semibold">{o.label}</b>
                    {o.description && <span className="block text-[12px] text-ink-3">{o.description}</span>}
                  </span>
                </button>
              );
            })
          )}
        </div>
      ))}
      {!answered && row.questions.some((q) => q.multi_select) && (
        <button className="btn btn-sm btn-primary justify-self-start" disabled={busy} onClick={() => void send(row.questions.map((_, i) => (picked[i] ?? []).join(", ")))}>
          Send answer
        </button>
      )}
      {!answered && <div className="text-[12px] text-ink-3">{live ? row.questions.some((q) => q.permission) ? "Waiting for your decision. Only the buttons above approve or deny this operation." : hint : "This request is no longer active. Resume the session to continue."}</div>}
    </div>
  );
}

