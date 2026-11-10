// "What it sees at start": the instructions file and the first prompt a session gets, in labelled parts that open
// one by one. Home builds them with the same functions as a real start (`preview` in `crates/api/src/bots.rs`), without starting
// anything: for a bot's next new session (`GET /api/bots/{id}/start-preview`) or a launch spec from the composer
// (`POST /api/launch/start-preview`).

import { useQuery } from "@tanstack/react-query";
import { Icon } from "@/components/Icon";
import { ErrorState, Loading } from "@/components/ui";
import type { StartPart } from "@/gen/StartPart";
import type { StartPreview } from "@/gen/StartPreview";
import { usePhone } from "@/lib/phone";
import { Sheet } from "@/mobile/Sheet";

const lines = (text: string) => text.trimEnd().split("\n").length;

function Part({ part }: { part: StartPart }) {
  const n = lines(part.text);
  return <details className="group rounded-control border border-line-2">
    <summary className="flex min-h-9 cursor-pointer list-none items-center gap-2 px-2.5 text-[13px] select-none">
      <Icon name="chev" size={12} className="flex-none -rotate-90 text-ink-3 transition-transform group-open:rotate-0" />
      <span className="min-w-0 flex-1 truncate font-semibold">{part.label}</span>
      <span className="flex-none text-[12px] text-ink-3 tabular-nums">{n} line{n === 1 ? "" : "s"}</span>
    </summary>
    <pre className="m-0 max-h-[50vh] overflow-auto border-t border-line-2 bg-sunk px-3 py-2 font-mono text-[12px] leading-5 whitespace-pre-wrap [overflow-wrap:anywhere]">{part.text}</pre>
  </details>;
}

function Parts({ title, hint, parts }: { title: string; hint: string; parts: StartPart[] }) {
  return <section className="grid gap-1.5">
    <div className="flex items-baseline gap-2"><h3 className="seclabel m-0">{title}</h3><span className="text-[12px] text-ink-3">{hint}</span></div>
    {parts.map((p) => <Part key={p.key} part={p} />)}
  </section>;
}

/** The dialog (a full sheet on the phone). `load` fetches the preview; `subject` names what starts. */
export function StartPreviewDialog({ queryKey, load, subject, onClose }: { queryKey: unknown[]; load: () => Promise<StartPreview>; subject: string; onClose: () => void }) {
  const phone = usePhone();
  const q = useQuery({ queryKey: ["start-preview", ...queryKey], queryFn: load, staleTime: 0, retry: false });
  const body = q.isLoading ? <Loading what="what it sees at start" />
    : q.error ? <ErrorState error={q.error} onRetry={() => void q.refetch()} />
    : q.data && <div className="grid gap-4">
      <Parts title="Instructions" hint="its instructions file, read before every turn" parts={q.data.instructions} />
      <Parts title="First prompt" hint="the first message of the session" parts={q.data.prompt} />
    </div>;
  const title = `What ${subject} sees at start`;
  if (phone) return <Sheet title={title} full onClose={onClose}><div className="p-3" data-tour="start-preview">{body}</div></Sheet>;
  return <div className="fixed inset-0 z-50 grid place-items-center bg-black/35 p-6" onClick={onClose} onKeyDown={(e) => { if (e.key === "Escape") onClose(); }}>
    <div role="dialog" aria-modal="true" aria-label={title} data-tour="start-preview" className="flex max-h-[calc(86vh/var(--lp-zoom,1))] w-[min(900px,calc(92vw/var(--lp-zoom,1)))] flex-col overflow-hidden rounded-card border border-line bg-paper shadow-[0_20px_60px_-20px_rgba(23,32,43,.6)]" onClick={(e) => e.stopPropagation()}>
      <div className="flex items-center gap-2 border-b border-line-2 px-4 py-2.5">
        <b className="min-w-0 flex-1 truncate text-[15px]">{title}</b>
        <button className="iconbtn" aria-label="Close" title="Close" onClick={onClose} autoFocus><Icon name="x" size={14} /></button>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto p-4">{body}</div>
    </div>
  </div>;
}
