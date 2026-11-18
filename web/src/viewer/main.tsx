// The folio viewer of a shared snapshot (a static Space: this page, its assets and folio.json). It renders the pages
// with the reader's renderer and theme, read-only: no comments, no data API (sketches show their default state), no
// problem boxes beyond what the renderer finds, no Launchpad API. Links to other folios stay plain text.

import { StrictMode, useEffect, useMemo, useState } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { FolioMark } from "@/gen/FolioMark";
import type { FolioPage } from "@/gen/FolioPage";
import type { FolioFiles } from "@/lib/folios";
import { FolioPageView, type MarkApi } from "@/features/folios/render";
import "./viewer.css";

type Snapshot = { title: string; slug: string; kind: string; status: string | null; summary: string | null; rev: number; shared_at: number; pages: FolioPage[]; files: Record<string, { text?: string; data?: string }>; marks: { kind: string; target: string | null; value: string | null; page: string | null }[] };

function Viewer() {
  const [snap, setSnap] = useState<Snapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [page, setPage] = useState<string>(new URLSearchParams(location.search).get("page") ?? "index.md");
  useEffect(() => { fetch("./folio.json").then((r) => (r.ok ? r.json() : Promise.reject(new Error(`folio.json: HTTP ${r.status}`)))).then(setSnap, (e: Error) => setError(e.message)); }, []);
  const files = useMemo<FolioFiles | null>(() => snap && {
    text: async (p) => { const f = snap.files[p]; if (f?.text == null) throw new Error(`${p} is not in this snapshot`); return f.text; },
    dataUrl: async (p) => { const f = snap.files[p]; if (!f?.data) throw new Error(`${p} is not in this snapshot`); return f.data; },
    data: null,
  }, [snap]);
  if (error) return <p className="p-5 text-[13px] text-danger">{error}</p>;
  if (!snap || !files) return <p className="p-5 text-[13px] text-ink-3">Loading…</p>;
  const current = snap.pages.find((p) => p.path === page) ?? snap.pages[0];
  const marks = snap.marks.map((m, i) => ({ id: i + 1, seq: i + 1, kind: m.kind, author: { kind: "owner", name: "Owner", session_id: null, model: null, runner: null }, page: m.page, target: m.target, quote: null, prefix: null, suffix: null, value: m.value, text: null, parent: null, rev: snap.rev, created_at: snap.shared_at, updated_at: snap.shared_at, sent_at: snap.shared_at, resolved_at: null, resolved_rev: null })) as unknown as FolioMark[];
  const api: MarkApi = {
    ws: "", slug: snap.slug, rev: snap.rev, page: current?.path ?? "index.md", marks, problems: [], readOnly: true, files, paths: Object.keys(snap.files),
    assetUrl: (p) => snap.files[p]?.data ?? null,
    setMark: () => {}, comment: () => {}, open: (p, anchor) => { setPage(p); if (anchor) requestAnimationFrame(() => document.getElementById(`f-${anchor}`)?.scrollIntoView()); }, openFolio: () => {},
  };
  return <div className="min-h-dvh bg-paper">
    <header className="flex flex-wrap items-center gap-2 border-b border-line px-5 py-2 text-[13px]">
      <b className="min-w-0 truncate">{snap.title}</b>
      <span className="text-[12px] text-ink-3">{snap.kind} · rev {snap.rev} · a read-only snapshot shared from Launchpad</span>
      {snap.pages.length > 1 && <nav className="ml-auto flex flex-wrap gap-1">{snap.pages.map((p) => <button key={p.path} type="button" className={`btn btn-sm ${p.path === current?.path ? "btn-primary" : ""}`} onClick={() => { setPage(p.path); scrollTo({ top: 0 }); }}>{p.title}</button>)}</nav>}
    </header>
    <main className="folio-frame px-5 py-6">
      {current?.path === snap.pages[0]?.path && <header className="folio-hero-head"><div className="folio-kicker">{[snap.kind, snap.status].filter(Boolean).join(" · ")}</div><h1>{snap.title}</h1>{snap.summary && <p className="folio-lede">{snap.summary}</p>}</header>}
      {current && <FolioPageView page={current} api={api} mood="report" />}
    </main>
  </div>;
}

createRoot(document.getElementById("root")!).render(<StrictMode><QueryClientProvider client={new QueryClient()}><Viewer /></QueryClientProvider></StrictMode>);
