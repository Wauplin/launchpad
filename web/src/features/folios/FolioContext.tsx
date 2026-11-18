// Folios as context of a launch or a message (`MessageContext.folios`): removable chips, and the Folio… picker of
// Add context (a folio, then the whole folio or one of its pages). The context is a reference: Home freezes the
// rev and the prompt gets the folio's name and the command that reads it, never the text.

import { useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { Folio } from "@/gen/Folio";
import type { FolioRef } from "@/gen/FolioRef";
import { Icon } from "@/components/Icon";
import { ago } from "@/lib/format";
import { folioRef, folioRefLabel, useFolio, useFolios } from "@/lib/folios";
import { usePhone } from "@/lib/phone";
import { folioKey } from "@/lib/promptDocument";
import { useScope } from "@/lib/workspace";

/** The folios of a context, as chips with ×. */
export function FolioContextChips({ folios = [], onRemove, disabled }: { folios?: FolioRef[]; onRemove: (key: string) => void; disabled?: boolean }) {
  return folios.map((f) => {
    const label = folioRefLabel(f);
    return <span key={folioKey(f)} title={`Folio${f.rev ? ` at rev ${f.rev}` : ""}: the agent reads it with launchpad folio read${f.quote ? ` · “${f.quote.slice(0, 80)}”` : ""}`} className="inline-flex h-7 min-w-0 max-w-full items-center gap-1 rounded-control border border-line bg-chrome pl-2 text-[12px] text-ink-2">
      <Icon name="folio" size={12} className="shrink-0" />
      <span className="truncate">{label}</span>
      {f.quote && <span className="shrink-0 text-ink-3">· quote</span>}
      <button className="iconbtn shrink-0" aria-label={`Remove folio ${label}`} title="Remove" disabled={disabled} onClick={() => onRemove(folioKey(f))}><Icon name="x" size={12} /></button>
    </span>;
  });
}

/** Folio…: find a folio of the workspace, then add it whole or one of its pages. */
export function FolioChoices({ onPick, close }: { onPick: (f: FolioRef) => void; close: () => void }) {
  const { workspace } = useScope();
  const list = useFolios(workspace.id);
  const [search, setSearch] = useState("");
  const [picked, setPicked] = useState<Folio | null>(null);
  const ref = useRef<HTMLDivElement>(null);
  const phone = usePhone();
  const needle = search.trim().toLocaleLowerCase();
  const rows = (list.data ?? []).filter((f) => !needle || `${f.title} ${f.slug} ${f.kind}`.toLocaleLowerCase().includes(needle));
  const virtual = useVirtualizer({ count: rows.length, getScrollElement: () => ref.current, estimateSize: () => (phone ? 44 : 40), overscan: 4, useFlushSync: false });
  if (picked) return <FolioParts folio={picked} onPick={(r) => { onPick(r); close(); }} back={() => setPicked(null)} />;
  return <div className="grid gap-1.5 p-2">
    <input autoFocus className="field-input w-full" aria-label="Find a folio" placeholder="Find a folio…" value={search} onChange={(e) => setSearch(e.target.value)}
      onKeyDown={(e) => { if (e.key === "Enter" && rows.length) { e.preventDefault(); setPicked(rows[0]); } }} />
    {rows.length > 0 && <div ref={ref} role="listbox" aria-label="Folios" className="max-h-64 overflow-y-auto"><div style={{ height: virtual.getTotalSize(), position: "relative" }}>{virtual.getVirtualItems().map((v) => {
      const f = rows[v.index];
      return <button key={f.id} type="button" role="option" aria-selected={v.index === 0 && !!needle} style={{ position: "absolute", top: v.start, height: v.size, left: 0 }} className="grid w-full grid-cols-[16px_minmax(0,1fr)] content-center gap-x-2 rounded-control px-2 text-left hover:bg-chrome" onClick={() => setPicked(f)}>
        <Icon name="folio" size={12} className="row-span-2 self-center text-ink-3" />
        <span className="truncate text-[13px]">{f.title}</span>
        <span className="truncate text-[11px] text-ink-3">{f.kind} · rev {f.rev} · {ago(f.updated_at)}</span>
      </button>;
    })}</div></div>}
    {list.isPending && <span className="px-2 text-[12px] text-ink-3">Loading folios…</span>}
    {list.error && <span className="px-2 text-[12px] text-danger-ink">{list.error.message}</span>}
    {list.data && rows.length === 0 && <span className="px-2 text-[12px] text-ink-3">{list.data.length ? "No folio matches." : "This workspace has no folio yet."}</span>}
  </div>;
}

/** The whole folio, or one page of it. */
function FolioParts({ folio, onPick, back }: { folio: Folio; onPick: (r: FolioRef) => void; back: () => void }) {
  const d = useFolio(folio.workspace_id, folio.slug);
  return <div className="grid gap-px p-1">
    <button type="button" className="flex min-h-8 items-center gap-2 rounded-control px-2 text-left text-[12px] text-ink-3 hover:bg-chrome" onClick={back}><Icon name="chev" size={12} className="rotate-90" />{folio.title}</button>
    <button type="button" className="flex min-h-8 items-center gap-2 rounded-control px-2 text-left text-[13px] hover:bg-chrome" onClick={() => onPick(folioRef(folio))}><Icon name="folio" size={12} />The whole folio<span className="ml-auto text-[12px] text-ink-3">rev {folio.rev}</span></button>
    {(d.data?.pages ?? []).length > 1 && d.data!.pages.map((p) => <button key={p.path} type="button" className="flex min-h-8 items-center gap-2 rounded-control px-2 text-left text-[13px] hover:bg-chrome" onClick={() => onPick(folioRef(folio, { page: p.path }))}><Icon name="file" size={12} /><span className="min-w-0 flex-1 truncate">{p.title}</span><span className="text-[12px] text-ink-3">{p.path}</span></button>)}
    {d.isLoading && <span className="px-2 text-[12px] text-ink-3">Loading pages…</span>}
  </div>;
}
