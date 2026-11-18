// The Folio tab of a session: the folios it wrote or worked on, in the reader (a picker when there are several).
// ⤢ opens the folio full screen at /folios/<slug>.

import { Link } from "@tanstack/react-router";
import { useState } from "react";
import type { Folio } from "@/gen/Folio";
import type { FolioActivity } from "@/gen/FolioActivity";
import { Icon } from "@/components/Icon";
import { ago } from "@/lib/format";
import { folioReceipt } from "@/lib/folios";
import { FolioReader, type Panel } from "./FolioReader";

export function SessionFolio({ folios }: { folios: Folio[] }) {
  const [slug, setSlug] = useState(folios[0]?.slug ?? "");
  const [nav, setNav] = useState<{ page?: string; rev?: number; panel?: Panel }>({});
  const f = folios.find((x) => x.slug === slug) ?? folios[0];
  if (!f) return null;
  return <div className="flex min-h-0 flex-1 flex-col" data-tour="session.folio">
    {folios.length > 1 && <div className="flex flex-wrap items-center gap-1.5 border-b border-line-2 px-3 py-1.5">
      {folios.map((x) => <button key={x.id} type="button" className={`btn btn-sm ${x.slug === f.slug ? "btn-primary" : ""}`} onClick={() => { setSlug(x.slug); setNav({}); }}>{x.title}</button>)}
    </div>}
    <div className="min-h-0 flex-1"><FolioReader key={f.id} ws={f.workspace_id} slug={f.slug} nav={nav} onNav={(n) => setNav((p) => ({ ...p, ...n }))} embedded /></div>
  </div>;
}

/** A folio write in a session's trace: "Updated folio Exit routes · rev 6 · 2 files", with Open. */
export function FolioWriteRow({ a }: { a: FolioActivity }) {
  return <div className="my-1 flex min-h-9 items-center gap-2 rounded-card border border-line-2 bg-chrome px-3 py-1.5 text-[13px]" data-tour="trace.folio">
    <Icon name="folio" size={12} className="shrink-0 text-ink-3" />
    <span className="min-w-0 flex-1 truncate" title={a.paths.join(", ")}>{folioReceipt(a).replace(/^./, (c) => c.toUpperCase())}<span className="text-ink-3"> · {a.paths.slice(0, 3).join(", ")}{a.paths.length > 3 ? "…" : ""} · {ago(a.at)}</span></span>
    <Link to="/folios/$slug" params={{ slug: a.slug }} className="btn btn-sm">Open</Link>
  </div>;
}
