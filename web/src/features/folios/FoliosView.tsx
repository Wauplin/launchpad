// The Folios page: the workspace's folios (Active, Archived), each with its kind, its author, its rev and what waits
// for you (unsent marks, open comments, problems). Open shows it in the reader; ⋯ archives, brings back or deletes.
// On the phone it is a page of More.

import { Link, useNavigate, useSearch } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useRef, useState } from "react";
import type { Folio } from "@/gen/Folio";
import { ActionMenuItem } from "@/components/ActionButton";
import { Icon } from "@/components/Icon";
import { Badge, ConfirmButton, Count, Empty, ErrorState, FilterChip, FilterChips, Loading, MenuSection, PageHeader, RowActions } from "@/components/ui";
import { toast } from "@/components/Toast";
import { runAction } from "@/lib/actions";
import { ago } from "@/lib/format";
import { useBlockRequests, useFolios } from "@/lib/folios";
import { usePhone } from "@/lib/phone";
import { useScope } from "@/lib/workspace";
import { PhoneHeader } from "@/mobile/PhoneHeader";
import { phoneBack } from "@/mobile/back";

/** Who wrote it last, from its links (sessions) or its creator. */
function byline(f: Folio): string {
  const last = f.links[0];
  const who = last?.title ? `“${last.title}”` : f.created_by.name;
  const agents = f.links.length;
  return [who, `rev ${f.rev}`, `updated ${ago(f.updated_at)}`, agents > 1 ? `${agents} agents` : null].filter(Boolean).join(" · ");
}

const FolioRow = memo(function FolioRow({ f, phone }: { f: Folio; phone: boolean }) {
  const ws = f.workspace_id;
  return <div className="group flex h-full min-w-0 items-center gap-2 border-b border-line-2 pr-2">
    <Link to="/folios/$slug" params={{ slug: f.slug }} className="flex h-full min-w-0 flex-1 flex-col justify-center gap-0.5 px-3 text-ink no-underline hover:bg-chrome" data-tour="folios.row">
      <span className="flex min-w-0 items-center gap-2">
        <Icon name="folio" size={12} className="shrink-0 text-ink-3" />
        <span className="min-w-0 truncate text-[13px] font-medium">{f.title}</span>
        {!phone && <Badge>{f.kind}</Badge>}
        {f.unsent > 0 && <Count n={f.unsent} title={`${f.unsent} marks not sent yet`} />}
        {f.share && <span className="shrink-0 text-[12px] text-teal-ink" title={`Shared as ${f.share.space}`}>{f.share.expires_at == null ? "shared" : `shared · ${Math.max(0, Math.ceil((f.share.expires_at - Date.now()) / 86_400_000))} days left`}</span>}
        {f.errors > 0 && <span className="shrink-0 text-[12px] text-danger" title="Problems of Home's check">{f.errors} error{f.errors === 1 ? "" : "s"}</span>}
      </span>
      <span className="truncate text-[12px] text-ink-3">{byline(f)}{f.open ? ` · ${f.open} open comment${f.open === 1 ? "" : "s"}` : ""}</span>
    </Link>
    <RowActions label="Folio actions" main={phone ? undefined : <Link to="/folios/$slug" params={{ slug: f.slug }} className="btn btn-sm">Open</Link>} more={(close) => <>
      <ActionMenuItem name="folio.archive" params={{ workspace: ws, slug: f.slug }} body={{ archived: f.state !== "archived" }} close={close} label={f.state === "archived" ? "Bring back" : "Archive"} detail={f.state === "archived" ? "With its files, revs and marks" : "Bring back from Archived"} onDone={() => toast(f.state === "archived" ? `${f.title} is back` : `${f.title} archived`)} />
      <MenuSection><ConfirmButton className="w-full" confirmLabel="Delete folio" consequence="Deletes this folio, its revs and your marks. Its topic and sessions stay." onConfirm={() => runAction("folio.delete", { workspace: ws, slug: f.slug }).then(() => { close(); toast("Folio deleted"); })}>Delete folio</ConfirmButton></MenuSection>
    </>} />
  </div>;
});

export function FoliosView() {
  const phone = usePhone();
  const scope = useScope();
  const navigate = useNavigate();
  const search = useSearch({ strict: false }) as { archived?: boolean; shared?: boolean };
  const archived = !!search.archived;
  const shared = !!search.shared && !archived;
  const q = useFolios(scope.workspace.id, archived);
  const active = useFolios(scope.workspace.id, false).data ?? [];
  const [needle, setNeedle] = useState("");
  const requests = useBlockRequests(scope.workspace.id, !archived).data ?? [];
  const list = (q.data ?? []).filter((f) => !shared || f.share).filter((f) => !needle || `${f.title} ${f.slug} ${f.kind}`.toLowerCase().includes(needle.toLowerCase()));
  const ref = useRef<HTMLDivElement>(null);
  const virtual = useVirtualizer({ count: list.length, getScrollElement: () => ref.current, estimateSize: () => (phone ? 60 : 52), overscan: 6, useFlushSync: false });
  const unsent = active.reduce((n, f) => n + f.unsent, 0);
  const sharedCount = active.filter((f) => f.share).length;
  const meta = <span>{active.length} active{sharedCount ? ` · ${sharedCount} shared` : ""}{unsent ? ` · ${unsent} unsent marks` : ""}</span>;
  const chips = <FilterChips label="Folios">
    <FilterChip active={!archived && !shared} onClick={() => void navigate({ to: "/folios", search: {} as never, replace: true })}>Active</FilterChip>
    <FilterChip active={shared} onClick={() => void navigate({ to: "/folios", search: { shared: true } as never, replace: true })}>Shared</FilterChip>
    <FilterChip active={archived} onClick={() => void navigate({ to: "/folios", search: { archived: true } as never, replace: true })}>Archived</FilterChip>
  </FilterChips>;
  return <div className="flex h-full min-h-0 flex-col" data-tour="folios.page">
    {phone
      ? <PhoneHeader title="Folios" onBack={() => phoneBack("/more")} subtitle={meta} />
      : <PageHeader title="Folios" meta={meta} intro="Folios are reports, plans, audits and sketchbooks that agents write as Markdown with typed blocks. Launchpad renders them, keeps every rev, and lets you comment, decide and mark items on the page; Send feedback delivers your marks to the folio's bot thread or the agent that wrote it. A folio with no change and no open mark for 30 days archives itself." />}
    <div className="flex min-h-0 w-full max-w-page flex-1 flex-col gap-2 px-5 py-3">
      <div className="flex flex-wrap items-center gap-2">{chips}{(q.data?.length ?? 0) > 6 && <input className="field-input min-w-0 flex-1" aria-label="Search folios" placeholder="Search folios…" value={needle} onChange={(e) => setNeedle(e.target.value)} />}</div>
      {q.isPending ? <Loading what="folios" /> : q.error ? <ErrorState error={q.error} /> : list.length === 0 ? <Empty icon="folio">{archived ? "No archived folios." : shared ? "No shared folio. Share one from its ⋯ (Share…)." : needle ? "No folio matches." : "No folios yet. Ask an agent to present its results as a folio (launchpad folio create)."}</Empty> : null}
      {requests.length > 0 && !archived && <details className="text-[12px] text-ink-3" data-tour="folios.requests"><summary className="cursor-pointer">Block requests · {requests.length}</summary>
        <p className="m-0 mt-1">Blocks and containers outside the catalog that agents used: candidates to ask an agent to add to the catalog.</p>
        <ul className="m-0 mt-1 grid gap-0.5 pl-4">{requests.map((r) => <li key={r.name}><code>{r.name}</code> · {r.folios} folio{r.folios === 1 ? "" : "s"}, used {r.uses} time{r.uses === 1 ? "" : "s"}</li>)}</ul>
      </details>}
      <div ref={ref} className="min-h-0 flex-1 overflow-auto"><div style={{ height: virtual.getTotalSize(), position: "relative" }}>{virtual.getVirtualItems().map((v) => <div key={v.key} style={{ position: "absolute", top: v.start, left: 0, width: "100%", height: v.size }}>
        <FolioRow f={list[v.index]} phone={phone} />
      </div>)}</div></div>
    </div>
  </div>;
}
