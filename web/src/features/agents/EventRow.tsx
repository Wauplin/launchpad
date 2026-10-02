// One display row of a session (see rows.ts). Memoized on the row version.

import { useQuery } from "@tanstack/react-query";
import { memo, useEffect, useRef, useState } from "react";
import { Icon } from "@/components/Icon";
import { Markdown } from "@/components/Markdown";
import { toast, toastError } from "@/components/Toast";
import { api } from "@/lib/api";
import { copyText } from "@/lib/clipboard";
import { formatAge } from "@/lib/format";
import { groupSummary, type Call, type Row } from "./rows";

/** "3 min" since `ts`, refreshed every 30 s (rows are memoized, so the time renders itself). */
function Ago({ ts }: { ts: number }) {
  const [, tick] = useState(0);
  useEffect(() => {
    const t = setInterval(() => tick((n) => n + 1), 30_000);
    return () => clearInterval(t);
  }, []);
  return <>{formatAge(ts)}</>;
}

function duration(ms: number | null | undefined) {
  if (ms == null) return "";
  if (ms < 1000) return `${ms} ms`;
  const s = Math.round(ms / 1000);
  return s < 60 ? `${s} s` : `${Math.floor(s / 60)} min ${s % 60} s`;
}

function AssistantRow({ text, streaming }: { text: string; streaming: boolean }) {
  const ref = useRef<HTMLDivElement>(null);
  return (
    <div className="group relative">
      {!streaming && (
        <div className="absolute -top-2 right-0 z-[2] hidden gap-1 group-hover:flex">
          <button className="btn btn-sm shadow-[0_1px_3px_rgba(23,32,43,.12)]" title="Copy the raw markdown" onClick={() => void copyText(text).then(() => toast("Copied"))}>
            <Icon name="copy" size={12} />
            Copy
          </button>
          <button
            className="btn btn-sm shadow-[0_1px_3px_rgba(23,32,43,.12)]"
            title="Copy the text without markdown"
            onClick={() => void copyText(ref.current?.innerText ?? text).then(() => toast("Copied as text"))}
          >
            Copy as text
          </button>
        </div>
      )}
      <div ref={ref}>
        <Markdown text={text} />
      </div>
    </div>
  );
}

function CallLine({ c }: { c: Call }) {
  const [open, setOpen] = useState(false);
  const running = !c.result;
  const out = c.result?.output ?? c.live ?? "";
  return (
    <div>
      <button
        onClick={() => setOpen((o) => !o)}
        className={`flex w-full items-center gap-[7px] rounded-md border px-2 py-1 text-left text-xs ${
          running ? "border-[#CFE0F4] bg-work-soft text-work" : c.result?.is_error ? "border-[#f3c7c7] bg-[#fff6f6] text-ink-2" : "border-line-2 bg-chrome text-ink-2"
        }`}
      >
        {running ? <span className="lp-spin" /> : <Icon name="chev" size={12} className={`flex-none text-ink-3 transition-transform ${open ? "" : "-rotate-90"}`} />}
        <span className={`font-[650] ${running ? "text-work" : "text-ink"}`}>{c.name}</span>
        <span className="min-w-0 truncate font-mono text-[11.5px]">{c.summary}</span>
        <span className="ml-auto flex-none whitespace-nowrap text-ink-3 tabular-nums">
          {running ? <Ago ts={c.ts} /> : [duration(c.result?.duration_ms), c.result?.exit_code != null ? `exit ${c.result.exit_code}` : c.result?.is_error ? "error" : ""].filter(Boolean).join(" · ")}
        </span>
      </button>
      {(open || (running && c.live)) && out && (
        <div className="relative mt-1">
          <pre className="m-0 max-h-72 overflow-auto rounded-md bg-[#1E242C] px-2.5 py-2 font-mono text-[11px] leading-relaxed whitespace-pre-wrap text-[#D6DEE8]">{out}</pre>
          <button className="btn btn-sm absolute top-1 right-1" onClick={() => void copyText(out).then(() => toast("Output copied"))}>
            <Icon name="copy" size={12} />
          </button>
        </div>
      )}
    </div>
  );
}

function ToolsRow({ calls }: { calls: Call[] }) {
  const [open, setOpen] = useState(false);
  const running = calls.filter((c) => !c.result);
  if (calls.length === 1) return <CallLine c={calls[0]} />;
  const total = calls.reduce((acc, c) => acc + (c.result?.duration_ms ?? 0), 0);
  return (
    <div className="grid gap-1">
      <button onClick={() => setOpen((o) => !o)} className="flex items-center gap-[7px] rounded-md border border-line-2 bg-chrome px-2 py-1 text-left text-xs text-ink-2">
        <Icon name="chev" size={12} className={`text-ink-3 transition-transform ${open ? "" : "-rotate-90"}`} />
        <span className="font-[650] text-ink">{groupSummary(calls)}</span>
        <span className="ml-auto text-ink-3 tabular-nums">{duration(total)}</span>
      </button>
      {open && (
        <div className="grid gap-1 pl-4">
          {calls.map((c) => (
            <CallLine key={c.id} c={c} />
          ))}
        </div>
      )}
      {!open && running.map((c) => <CallLine key={c.id} c={c} />)}
    </div>
  );
}

function EditRow({ path, change, diff }: { path: string; change: string; diff: string }) {
  const [open, setOpen] = useState(false);
  const lines = diff.split("\n");
  const add = lines.filter((l) => l.startsWith("+")).length;
  const del = lines.filter((l) => l.startsWith("-")).length;
  const shown = open ? lines : lines.slice(0, 12);
  return (
    <div className="overflow-hidden rounded-md border border-line-2">
      <div className="flex items-center gap-[7px] bg-chrome px-2 py-1 text-xs text-ink-2">
        <Icon name={change === "add" ? "file" : "pencil"} size={12} />
        <span className="min-w-0 truncate font-mono">{path}</span>
        {change === "add" && <span className="text-ink-3">new file</span>}
        <span className="font-[650] text-gh-open">+{add}</span>
        <span className="font-[650] text-gh-closed">−{del}</span>
        <button className="ml-auto text-ink-3 hover:text-ink" title="Copy the diff" onClick={() => void copyText(diff).then(() => toast("Diff copied"))}>
          <Icon name="copy" size={12} />
        </button>
      </div>
      <pre className="m-0 overflow-x-auto py-1 font-mono text-[11px] leading-[1.55] whitespace-pre">
        {shown.map((l, i) => (
          <span key={i} className={`block px-2 ${l.startsWith("+") ? "bg-[#E8F6EC] text-[#0F4D23]" : l.startsWith("-") ? "bg-[#FCEBEC] text-[#7D1A20]" : ""}`}>
            {l || " "}
          </span>
        ))}
      </pre>
      {lines.length > 12 && (
        <button className="w-full border-t border-line-2 bg-chrome py-0.5 text-[11px] text-ink-3 hover:text-ink" onClick={() => setOpen((o) => !o)}>
          {open ? "Show less" : `Show ${lines.length - 12} more lines`}
        </button>
      )}
    </div>
  );
}

function PlanRow({ items }: { items: Extract<Row, { t: "plan" }>["items"] }) {
  return (
    <div className="grid gap-[3px] rounded-md border border-line-2 px-[9px] py-[7px] text-xs">
      {items.map((it, i) => (
        <div
          key={i}
          className={`flex items-center gap-[7px] ${it.status === "completed" ? "text-ink-3 line-through" : it.status === "in_progress" ? "font-semibold text-ink" : "text-ink-2"}`}
        >
          <span
            className={`size-3 flex-none rounded-[3px] border-[1.5px] ${
              it.status === "completed" ? "border-ink-3 bg-ink-3" : it.status === "in_progress" ? "border-work bg-work-soft" : "border-ink-3"
            }`}
          />
          {it.text}
        </div>
      ))}
    </div>
  );
}

export function AskCard({ row, sessionId, live }: { row: Extract<Row, { t: "ask" }>; sessionId: string; live: boolean }) {
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
    <div className={`grid gap-2 rounded-lg border px-2.5 py-[9px] ${answered ? "border-line-2 bg-chrome" : "border-attn-line bg-[#FFFBEF]"}`}>
      {row.questions.map((q, qi) => (
        <div key={qi} className="grid gap-1.5">
          <div className="text-[12.5px] font-[650]">{q.question}</div>
          {answered ? (
            <div className="text-[12.5px] text-ink-2">
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
                  className={`flex items-start gap-2 rounded-md border px-2 py-1.5 text-left text-[12.5px] hover:border-teal disabled:opacity-60 ${on ? "border-teal bg-teal-soft" : "border-line bg-paper"}`}
                >
                  <span className="font-mono text-[11px] text-ink-3">{oi + 1}</span>
                  <span>
                    <b className="font-semibold">{o.label}</b>
                    {o.description && <span className="block text-[11.5px] text-ink-3">{o.description}</span>}
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
      {!answered && <div className="text-[11.5px] text-ink-3">{live ? "Or type another answer below." : "The session process stopped: resume it to answer."}</div>}
    </div>
  );
}

function BackgroundRow({ row, sessionId }: { row: Extract<Row, { t: "bg" }>; sessionId: string }) {
  const running = row.status === "running";
  const out = useQuery({
    queryKey: ["task-output", sessionId, row.task_id, row.status],
    queryFn: () => api.get<{ text: string | null }>(`/api/sessions/${sessionId}/task-output?task_id=${encodeURIComponent(row.task_id)}`),
    enabled: !!row.output_file,
    refetchInterval: running ? 3000 : false,
  });
  const tail = (out.data?.text ?? "").split("\n").slice(-8).join("\n").trimEnd();
  return (
    <div className="overflow-hidden rounded-md border border-attn-line">
      <div className="flex items-center gap-[7px] bg-attn-soft px-2 py-1 text-xs text-attn">
        {running ? <span className="lp-spin border-attn-line! border-t-attn!" /> : <Icon name="check" size={12} />}
        <span className="font-[650]">Background</span>
        <span className="min-w-0 truncate font-mono text-ink-2">{row.description}</span>
        <span className="ml-auto whitespace-nowrap tabular-nums">{running ? <Ago ts={row.ts} /> : row.status}</span>
      </div>
      {tail && <pre className="m-0 max-h-40 overflow-auto bg-[#1E242C] px-2.5 py-2 font-mono text-[11px] leading-relaxed whitespace-pre-wrap text-[#D6DEE8]">{tail}</pre>}
    </div>
  );
}

function Separator({ children, tone = "" }: { children: React.ReactNode; tone?: string }) {
  return (
    <div className={`flex items-center gap-2 text-[11.5px] text-ink-3 before:h-px before:flex-1 before:bg-line-2 after:h-px after:flex-1 after:bg-line-2 ${tone}`}>{children}</div>
  );
}

export const EventRow = memo(
  function EventRow({ row, sessionId, live }: { row: Row; sessionId: string; live: boolean }) {
    switch (row.t) {
      case "user":
        return (
          <div className="ml-auto max-w-[88%] rounded-[10px_10px_3px_10px] bg-sunk px-2.5 py-[7px] text-[12.5px] whitespace-pre-wrap">
            {row.text}
            {row.queued && <span className="ml-2 text-[11px] text-ink-3">queued: read after the current step</span>}
          </div>
        );
      case "assistant":
        return <AssistantRow text={row.text} streaming={row.streaming} />;
      case "reasoning":
        return <div className="border-l-2 border-line pl-2 text-xs text-ink-3 italic">{row.text}</div>;
      case "tools":
        return <ToolsRow calls={row.calls} />;
      case "edit":
        return <EditRow path={row.path} change={row.change} diff={row.diff} />;
      case "plan":
        return <PlanRow items={row.items} />;
      case "ask":
        return <AskCard row={row} sessionId={sessionId} live={live} />;
      case "bg":
        return <BackgroundRow row={row} sessionId={sessionId} />;
      case "turn":
        return (
          <Separator tone={row.error ? "text-gh-closed" : ""}>
            {row.error ? `Turn failed: ${row.error}` : row.interrupted ? "Interrupted" : "Turn finished"} · <Ago ts={row.ts} /> ago
          </Separator>
        );
      case "notice":
        return <Separator>{row.text}</Separator>;
      case "error":
        return <div className="rounded-md border border-[#f3c7c7] bg-[#fff6f6] px-2.5 py-1.5 font-mono text-xs whitespace-pre-wrap text-gh-closed">{row.text}</div>;
    }
  },
  (a, b) => a.row.key === b.row.key && a.row.v === b.row.v && a.live === b.live && a.sessionId === b.sessionId,
);
