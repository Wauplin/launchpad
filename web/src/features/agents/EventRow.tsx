import { daemonUrl } from "@/lib/connection";
// One display row of a session (see rows.ts). Memoized on the row version.

import { useQuery } from "@tanstack/react-query";
import { memo, useContext, useEffect, useRef, useState } from "react";
import { Icon } from "@/components/Icon";
import { CodeLinksContext, Markdown } from "@/components/Markdown";
import { toast } from "@/components/Toast";
import { api } from "@/lib/api";
import { copyText } from "@/lib/clipboard";
import { formatAge } from "@/lib/format";
import { groupSummary, type Call, type Row } from "./rows";
import { noticesSummary, type TraceRow } from "./traceRows";
import { AskCard } from "./AskCard";
export { AskCard };

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
        <div className="absolute -top-2 right-0 z-[2] hidden gap-1 group-hover:flex [@media(hover:none)]:static [@media(hover:none)]:mb-1 [@media(hover:none)]:flex [@media(hover:none)]:justify-end">
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
        <Markdown text={text} runShell={!streaming} />
      </div>
    </div>
  );
}

/** A file path that opens the code viewer when it is a file of the session's folder. */
function PathText({ path, className }: { path: string; className: string }) {
  const links = useContext(CodeLinksContext);
  const hit = links?.resolve(path);
  if (!hit) return <span className={className}>{path}</span>;
  return (
    <span
      role="link"
      className={`${className} lp-path`}
      title="Open in Files"
      onClick={(e) => {
        e.stopPropagation();
        links!.open(hit.path, hit.line);
      }}
    >
      {path}
    </span>
  );
}

const FILE_TOOLS = new Set(["Read", "Edit", "Write", "MultiEdit", "NotebookEdit", "View image"]);

function CallLine({ c }: { c: Call }) {
  const [open, setOpen] = useState(false);
  const running = !c.result;
  const out = c.result?.output ?? c.live ?? "";
  return (
    <div>
      <button
        onClick={() => setOpen((o) => !o)}
        className={`flex w-full items-center gap-[7px] rounded-md border px-2 py-1 text-left text-xs ${
          running ? "border-work/30 bg-work-soft text-work" : c.result?.is_error ? "border-gh-closed/30 bg-diff-del text-ink-2" : "border-line-2 bg-chrome text-ink-2"
        }`}
      >
        {running ? <span className="lp-spin" /> : <Icon name="chev" size={12} className={`flex-none text-ink-3 transition-transform ${open ? "" : "-rotate-90"}`} />}
        <span className={`font-[650] ${running ? "text-work" : "text-ink"}`}>{c.name}</span>
        {FILE_TOOLS.has(c.name) ? <PathText path={c.summary} className="min-w-0 truncate font-mono text-[12px]" /> : <span className="min-w-0 truncate font-mono text-[12px]">{c.summary}</span>}
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
        <PathText path={path} className="min-w-0 truncate font-mono" />
        {change === "add" && <span className="text-ink-3">new file</span>}
        <span className="font-[650] text-gh-open">+{add}</span>
        <span className="font-[650] text-gh-closed">−{del}</span>
        <button className="ml-auto text-ink-3 hover:text-ink" title="Copy the diff" onClick={() => void copyText(diff).then(() => toast("Diff copied"))}>
          <Icon name="copy" size={12} />
        </button>
      </div>
      <pre className="m-0 overflow-x-auto py-1 font-mono text-[11px] leading-[1.55] whitespace-pre">
        {shown.map((l, i) => (
          <span key={i} className={`block px-2 ${l.startsWith("+") ? "bg-diff-add text-gh-open" : l.startsWith("-") ? "bg-diff-del text-gh-closed" : ""}`}>
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

/** Appended context, presets and Launchpad instructions of the launch prompt, folded. */
export function Instructions({ blocks, className = "", label }: { blocks: string[]; className?: string; label?: string }) {
  const [open, setOpen] = useState(false);
  return <div className={className}>
    <button className="text-[12px] text-ink-3 hover:text-ink" aria-expanded={open} onClick={() => setOpen((o) => !o)}>{open ? "−" : "+"} {label ?? `Launchpad instructions (${blocks.length})`}</button>
    {open && <div className="mt-1 grid gap-2 border-l-2 border-line pl-2 text-xs text-ink-2">{blocks.map((b, i) => <div key={i}>{b}</div>)}</div>}
  </div>;
}

function NoticesRow({ texts }: { texts: string[] }) {
  const [open, setOpen] = useState(false);
  return <div className="grid gap-1">
    <Separator><button className="hover:text-ink" aria-expanded={open} onClick={() => setOpen((o) => !o)}>{noticesSummary(texts)} {open ? "▾" : "▸"}</button></Separator>
    {open && texts.map((t, i) => <div key={i} className="text-center text-[12px] text-ink-3">{t}</div>)}
  </div>;
}

function Separator({ children, tone = "" }: { children: React.ReactNode; tone?: string }) {
  return (
    <div className={`flex items-center gap-2 text-[12px] text-ink-3 before:h-px before:flex-1 before:bg-line-2 after:h-px after:flex-1 after:bg-line-2 ${tone}`}>{children}</div>
  );
}

export const EventRow = memo(
  function EventRow({ row, sessionId, live }: { row: TraceRow; sessionId: string; live: boolean }) {
    switch (row.t) {
      case "user":
        return (
          <div className="ml-auto max-w-[88%] rounded-[10px_10px_3px_10px] bg-sunk px-2.5 py-[7px] text-[13px] whitespace-pre-wrap">
            {row.text}
            {row.instructions && <Instructions blocks={row.instructions} className={row.text ? "mt-1.5" : ""} />}
            {row.presets && <Instructions label={`Presets: ${row.presets.map((p) => p.label).join(" · ")}`} blocks={row.presets.map((p) => p.text.trim())} className={row.text ? "mt-1.5" : ""} />}
            <div className="flex flex-wrap gap-2">{row.images.map((image, i) => <a key={i} href={image.data_url.startsWith("/api/") ? daemonUrl(image.data_url) : image.data_url} download={image.name} title={`Save ${image.name}`}><img src={image.data_url.startsWith("/api/") ? daemonUrl(image.data_url) : image.data_url} alt={image.name} className="mt-2 max-h-40 max-w-full rounded-md border border-line" /></a>)}</div>
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
            <span title={new Date(row.ts).toLocaleString()}>{row.error ? `Turn failed: ${row.error}` : row.interrupted ? "Turn interrupted" : "Turn ended"} · <Ago ts={row.ts} /> ago</span>
          </Separator>
        );
      case "notice":
        return <Separator>{row.text}</Separator>;
      case "notices":
        return <NoticesRow texts={row.texts} />;
      case "error":
        return <div className="rounded-md border border-gh-closed/30 bg-diff-del px-2.5 py-1.5 font-mono text-xs whitespace-pre-wrap text-gh-closed">{row.text}</div>;
    }
  },
  (a, b) => a.row.key === b.row.key && a.row.v === b.row.v && a.live === b.live && a.sessionId === b.sessionId,
);
