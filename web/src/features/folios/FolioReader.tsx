// The folio reader: /folios/<slug> full screen (the laptop hides the sidebar), and the Folio tab of a session
// (`embedded`). A slim bar (Folios, the title, its rev, problems, Comments, Send feedback, ⋯), the contents on the
// left, the page in the middle and one side panel on the right (comments, versions or problems; a sheet on the phone).
// Marks (comments, verdicts, choices, ticks) are saved on Home at once and stay unsent until Send feedback; the address
// keeps the page, the rev and the panel (`?page=&rev=&panel=`).

import { useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate, useParams, useSearch } from "@tanstack/react-router";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { FolioDetail } from "@/gen/FolioDetail";
import type { FolioMark } from "@/gen/FolioMark";
import { ActionMenuItem } from "@/components/ActionButton";
import { Icon } from "@/components/Icon";
import { MenuItem, Popover } from "@/components/Popover";
import { ConfirmButton, Empty, ErrorState, Loading, MenuSection, MoreMenu, humanError } from "@/components/ui";
import { toast, toastError } from "@/components/Toast";
import { runAction } from "@/lib/actions";
import { useBots } from "@/lib/bots";
import { ago, when } from "@/lib/format";
import { marksSummary, unsentMarks, useFolio, useFolioDiff, useFolioRevs, useFolioTargets, VERDICT_LABEL } from "@/lib/folios";
import { usePhone } from "@/lib/phone";
import { useScope } from "@/lib/workspace";
import { PhoneHeader } from "@/mobile/PhoneHeader";
import { Sheet } from "@/mobile/Sheet";
import { phoneBack } from "@/mobile/back";
import { FolioPageView, type MarkApi } from "./render";

export type Panel = "comments" | "versions" | "problems";
type Nav = { page?: string; rev?: number; panel?: Panel };
type Draft = { target: string; page: string; quote?: string; prefix?: string; suffix?: string; text: string };

/** The route `/folios/$slug`. */
export function FolioRoute() {
  const { slug } = useParams({ strict: false }) as { slug: string };
  const search = useSearch({ strict: false }) as Nav;
  const navigate = useNavigate();
  const scope = useScope();
  const onNav = useCallback((n: Nav) => void navigate({ to: "/folios/$slug", params: { slug }, search: (prev: Nav) => clean({ ...prev, ...n }), replace: true } as never), [navigate, slug]);
  return <FolioReader ws={scope.workspace.id} slug={slug} nav={search} onNav={onNav} />;
}

const clean = (n: Nav): Nav => Object.fromEntries(Object.entries(n).filter(([, v]) => v != null && v !== "")) as Nav;

export function FolioReader({ ws, slug, nav, onNav, embedded = false }: { ws: string; slug: string; nav: Nav; onNav: (n: Nav) => void; embedded?: boolean }) {
  const phone = usePhone();
  const navigate = useNavigate();
  const q = useFolio(ws, slug, nav.rev ?? null);
  const latest = useFolio(ws, slug, null);
  const d = q.data;
  const qc = useQueryClient();
  const [draft, setDraft] = useState<Draft | null>(null);
  const [toc, setToc] = useState(false);
  const scroller = useRef<HTMLDivElement>(null);
  const pagePath = nav.page && d?.pages.some((p) => p.path === nav.page) ? nav.page : d?.pages[0]?.path ?? "index.md";
  const page = d?.pages.find((p) => p.path === pagePath) ?? null;
  const readOnly = nav.rev != null && nav.rev !== latest.data?.folio.rev;
  const panel = nav.panel ?? null;
  const setPanel = (p: Panel | null) => onNav({ panel: p ?? undefined });
  const refresh = useCallback(() => void qc.invalidateQueries({ queryKey: ["folio", ws, slug] }), [qc, ws, slug]);

  const api = useMemo<MarkApi | null>(() => !d ? null : {
    ws, slug, rev: nav.rev ?? null, page: pagePath, marks: d.marks, problems: d.problems, readOnly,
    setMark: (kind, target, value) => void runAction("folio.mark", { workspace: ws, slug }, { kind, target, page: pagePath, value: value ?? "" }).then(refresh).catch(toastError),
    comment: (target, quote) => { setDraft({ target, page: pagePath, text: "", ...quote }); setPanel("comments"); },
    open: (p, anchor) => { onNav({ page: p }); if (anchor) requestAnimationFrame(() => document.getElementById(`f-${anchor}`)?.scrollIntoView({ block: "start" })); },
    openFolio: (other) => void navigate({ to: "/folios/$slug", params: { slug: other } }),
  }, [d, ws, slug, nav.rev, pagePath, readOnly, refresh, onNav, navigate]); // eslint-disable-line react-hooks/exhaustive-deps

  // A selection in the page offers Comment, with the quote and its surroundings (so the comment survives edits).
  const [selection, setSelection] = useState<{ x: number; y: number; target: string; quote: string; prefix: string; suffix: string } | null>(null);
  const onSelect = () => {
    const sel = window.getSelection();
    const text = sel?.toString().trim() ?? "";
    if (!sel || sel.rangeCount === 0 || text.length < 3 || readOnly) { setSelection(null); return; }
    const range = sel.getRangeAt(0);
    const node = range.startContainer;
    const el = (node.nodeType === 3 ? node.parentElement : node as Element) ?? null;
    if (!el || !scroller.current?.contains(el)) { setSelection(null); return; }
    const target = el.closest("[data-folio-target]")?.getAttribute("data-folio-target") ?? precedingAnchor(scroller.current, el) ?? "";
    const whole = node.textContent ?? "";
    const r = range.getBoundingClientRect();
    setSelection({ x: r.left + r.width / 2, y: r.top, target, quote: text.slice(0, 400), prefix: whole.slice(Math.max(0, range.startOffset - 30), range.startOffset), suffix: node === range.endContainer ? whole.slice(range.endOffset, range.endOffset + 30) : "" });
  };
  useEffect(() => { const clear = () => setSelection(null); window.addEventListener("scroll", clear, true); return () => window.removeEventListener("scroll", clear, true); }, []);

  if (q.isLoading) return <Loading what="folio" className="h-full" />;
  if (q.error || !d) return <ErrorState error={q.error ?? new Error("no folio")} onRetry={() => void q.refetch()} className="h-full" />;
  const f = d.folio;
  const unsent = unsentMarks(d.marks);
  const errors = d.problems.filter((p) => p.severity === "error").length;
  const warnings = d.problems.filter((p) => p.severity === "warn").length;
  const mood = (front(d.pages[0]?.text ?? "").mood === "editorial" ? "editorial" : "report") as "report" | "editorial";
  // The contents list: headings of levels 2 and 3 outside containers (inside, they title cards, tabs and boxes).
  const boxes = page ? page.outline.items.filter((i) => i.t === "container") as { line: number; end: number }[] : [];
  const headings = page ? page.outline.items.filter((i) => i.t === "heading" && i.level >= 2 && i.level <= 3 && !boxes.some((b) => i.line > b.line && i.line < b.end)) as { t: "heading"; line: number; level: number; id: string; text: string }[] : [];
  const jump = (id: string) => { document.getElementById(`f-${id}`)?.scrollIntoView({ block: "start", behavior: "smooth" }); setToc(false); };

  const contents = <nav aria-label="Contents" className="folio-toc">
    {d.pages.length > 1 && <>
      <div className="folio-toc-label">Pages</div>
      {d.pages.map((p) => <button key={p.path} type="button" className={p.path === pagePath ? "on" : ""} onClick={() => { onNav({ page: p.path }); setToc(false); scroller.current?.scrollTo({ top: 0 }); }}>{p.title}</button>)}
      <div className="folio-toc-label">On this page</div>
    </>}
    {headings.map((h, i) => <button key={h.id} type="button" className={h.level === 3 ? "sub" : ""} onClick={() => jump(h.id)}><span className="folio-toc-n">{h.level === 2 ? String(headings.filter((x, k) => k <= i && x.level === 2).length).padStart(2, "0") : ""}</span>{h.text}</button>)}
    {!headings.length && d.pages.length <= 1 && <span className="text-[12px] text-ink-3">No sections</span>}
  </nav>;

  const sideBody = (p: Panel) => p === "comments" ? <CommentsPanel d={d} ws={ws} slug={slug} draft={draft} setDraft={setDraft} readOnly={readOnly} refresh={refresh} jump={(m) => { if (m.page && m.page !== pagePath) onNav({ page: m.page }); if (m.target) requestAnimationFrame(() => jump(m.target!)); }} />
    : p === "versions" ? <VersionsPanel ws={ws} slug={slug} current={nav.rev ?? null} latest={latest.data?.folio.rev ?? f.rev} view={(rev) => onNav({ rev: rev ?? undefined })} />
    : <ProblemsPanel d={d} ws={ws} slug={slug} readOnly={readOnly} jump={(file, line) => { if (file !== pagePath && d.pages.some((x) => x.path === file)) onNav({ page: file }); void line; }} />;
  const PANEL_TITLE: Record<Panel, string> = { comments: "Comments", versions: "Versions", problems: "Problems" };

  const send = <SendFeedback ws={ws} slug={slug} unsent={unsent} />;
  const menu = (close: () => void) => <>
    <MenuItem icon={<Icon name="comment" size={14} />} label="Comments" detail={`${f.open} open`} onClick={() => { close(); setPanel("comments"); }} />
    <MenuItem icon={<Icon name="clock" size={14} />} label="Versions" detail={`rev ${f.rev}`} onClick={() => { close(); setPanel("versions"); }} />
    {d.problems.length > 0 && <MenuItem icon={<Icon name="alert" size={14} />} label="Problems" detail={`${errors} error${errors === 1 ? "" : "s"}, ${warnings} warning${warnings === 1 ? "" : "s"}`} onClick={() => { close(); setPanel("problems"); }} />}
    {embedded && <MenuItem icon={<Icon name="expand" size={14} />} label="Open full screen" onClick={() => { close(); void navigate({ to: "/folios/$slug", params: { slug } }); }} />}
    {f.topic_id && <MenuItem icon={<Icon name="tag" size={14} />} label="Agents on this folio" detail="Its topic" onClick={() => { close(); void navigate({ to: "/topics/$id", params: { id: f.topic_id! } }); }} />}
    <ActionMenuItem name="folio.archive" params={{ workspace: ws, slug }} body={{ archived: f.state !== "archived" }} close={close} label={f.state === "archived" ? "Bring back" : "Archive"} onDone={() => toast(f.state === "archived" ? "Folio brought back" : "Folio archived: Bring back in Folios › Archived")} />
    <MenuSection><ConfirmButton className="w-full" confirmLabel="Delete folio" consequence="Deletes this folio, its revs and your marks. Its topic and sessions stay." onConfirm={() => runAction("folio.delete", { workspace: ws, slug }).then(() => { close(); toast("Folio deleted"); void navigate({ to: "/folios" }); })}>Delete folio</ConfirmButton></MenuSection>
  </>;

  const meta = <>{f.kind} · rev {d.rev}{readOnly ? " (older)" : ""} · {ago(f.updated_at)}{f.state === "archived" ? " · archived" : ""}</>;
  const problemsButton = d.problems.some((p) => p.severity !== "info") && <button type="button" className={`btn btn-sm ${errors ? "text-danger" : ""}`} onClick={() => setPanel(panel === "problems" ? null : "problems")} title="The problems of Home's check of this rev"><Icon name="alert" size={12} />{errors + warnings} problem{errors + warnings === 1 ? "" : "s"}</button>;

  const body = <div ref={scroller} className="min-h-0 flex-1 overflow-y-auto" onMouseUp={onSelect} onTouchEnd={() => setTimeout(onSelect, 50)}>
    {readOnly && <div className="flex flex-wrap items-center gap-2 border-b border-attn-line bg-attn-soft px-5 py-1.5 text-[12px] text-attn">Rev {nav.rev}, read-only.<button type="button" className="btn btn-sm" onClick={() => onNav({ rev: undefined })}>Back to the latest</button></div>}
    <div className={`folio-frame ${phone ? "px-4 py-4" : "px-8 py-6"}`}>
      {page && api && <>
        {page.path === d.pages[0]?.path && <Hero d={d} />}
        <FolioPageView page={page} api={api} mood={mood} />
      </>}
      {!page && <Empty icon="folio">This folio has no pages yet.</Empty>}
      {d.pages.length > 1 && page && <PageSteps pages={d.pages.map((p) => ({ path: p.path, title: p.title }))} current={page.path} go={(p) => { onNav({ page: p }); scroller.current?.scrollTo({ top: 0 }); }} />}
    </div>
    {selection && <button type="button" className="btn btn-sm btn-primary fixed z-40 -translate-x-1/2 -translate-y-full shadow-lg" style={{ left: selection.x, top: selection.y - 6 }} onMouseDown={(e) => e.preventDefault()} onClick={() => { setDraft({ target: selection.target, page: pagePath, quote: selection.quote, prefix: selection.prefix, suffix: selection.suffix, text: "" }); setPanel("comments"); setSelection(null); window.getSelection()?.removeAllRanges(); }}><Icon name="comment" size={12} />Comment</button>}
  </div>;

  if (phone) return <div className="flex h-full min-h-0 flex-col bg-paper" data-tour="folio.reader">
    {!embedded && <PhoneHeader title={f.title} onBack={() => phoneBack("/folios")} subtitle={<span>{meta}</span>} primary={send} actions={(close) => <div className="grid">{menu(close)}</div>} />}
    {embedded && <div className="flex items-center gap-2 border-b border-line px-3 py-1.5 text-[12px] text-ink-3"><span className="min-w-0 flex-1 truncate">{meta}</span>{send}<MoreMenu label="Folio actions" width={260}>{menu}</MoreMenu></div>}
    <button type="button" className="flex min-h-11 items-center gap-2 border-b border-line-2 px-4 text-left text-[13px] text-ink-2" onClick={() => setToc(true)}><Icon name="sidebar" size={14} />Contents{page && d.pages.length > 1 ? ` · ${page.title}` : ""}</button>
    {body}
    {toc && <Sheet title="Contents" onClose={() => setToc(false)}><div className="p-2">{contents}</div></Sheet>}
    {panel && <Sheet title={PANEL_TITLE[panel]} full onClose={() => setPanel(null)}>{sideBody(panel)}</Sheet>}
  </div>;

  return <div className="flex h-full min-h-0 flex-col bg-paper" data-tour="folio.reader">
    <div className="flex min-h-[44px] flex-none flex-wrap items-center gap-2 border-b border-line px-5 py-1.5">
      {!embedded && <Link to="/folios" className="text-[13px] text-ink-3 no-underline hover:text-ink">Folios</Link>}
      {!embedded && <span className="text-ink-3">/</span>}
      <b className="min-w-0 truncate text-[13px]" title={f.title}>{f.title}</b>
      <span className="text-[12px] text-ink-3 whitespace-nowrap">{meta}</span>
      <span className="ml-auto flex items-center gap-1.5">
        {problemsButton}
        <button type="button" className="btn btn-sm" aria-pressed={panel === "comments"} onClick={() => setPanel(panel === "comments" ? null : "comments")}><Icon name="comment" size={12} />Comments{f.open ? ` · ${f.open}` : ""}</button>
        {send}
        {embedded && <Link to="/folios/$slug" params={{ slug }} className="iconbtn" title="Open full screen" aria-label="Open full screen"><Icon name="expand" size={14} /></Link>}
        <MoreMenu label="Folio actions" width={240}>{menu}</MoreMenu>
      </span>
    </div>
    <div className="flex min-h-0 flex-1">
      {!embedded && <aside className="hidden w-[210px] flex-none overflow-y-auto border-r border-line-2 px-2 py-4 lg:block">{contents}</aside>}
      {body}
      {panel && <aside className="flex w-[320px] flex-none flex-col border-l border-line bg-chrome" data-tour={panel === "comments" ? "folio.comments" : undefined}>
        <div className="flex items-center gap-2 border-b border-line-2 py-1 pr-1 pl-3"><b className="flex-1 text-[13px]">{PANEL_TITLE[panel]}</b><button type="button" className="iconbtn" aria-label="Close panel" title="Close panel" onClick={() => setPanel(null)}><Icon name="x" size={14} /></button></div>
        <div className="min-h-0 flex-1 overflow-y-auto">{sideBody(panel)}</div>
      </aside>}
    </div>
  </div>;
}

/** The last heading or target before an element, for a selection outside any block. */
function precedingAnchor(root: HTMLElement, el: Element): string | null {
  let found: string | null = null;
  for (const h of root.querySelectorAll("[data-anchor]")) {
    if (h.compareDocumentPosition(el) & Node.DOCUMENT_POSITION_FOLLOWING) found = h.getAttribute("data-anchor");
  }
  return found;
}

/** The front matter of index.md, read loosely (title, kind, status, summary, pills, mood). */
function front(text: string): Record<string, string> {
  const m = /^---\r?\n([\s\S]*?)\r?\n(?:---|\.\.\.)\s*(?:\r?\n|$)/.exec(text);
  const out: Record<string, string> = {};
  for (const l of (m?.[1] ?? "").split(/\r?\n/)) {
    const kv = /^([a-z_]+):\s*(.*)$/.exec(l);
    if (kv) out[kv[1]] = kv[2].replace(/^["']|["']$/g, "");
  }
  return out;
}

function Hero({ d }: { d: FolioDetail }) {
  const fm = front(d.pages[0]?.text ?? "");
  const pills = (fm.pills ?? "").replace(/^\[|\]$/g, "").split(",").map((s) => s.trim().replace(/^["']|["']$/g, "")).filter(Boolean);
  return <header className="folio-hero-head">
    <div className="folio-kicker">{[d.folio.kind, d.folio.status].filter(Boolean).join(" · ")}</div>
    <h1>{d.folio.title}</h1>
    {d.folio.summary && <p className="folio-lede">{d.folio.summary}</p>}
    {pills.length > 0 && <div className="folio-pills">{pills.map((p) => <span key={p} className="folio-pill">{p}</span>)}</div>}
  </header>;
}

function PageSteps({ pages, current, go }: { pages: { path: string; title: string }[]; current: string; go: (p: string) => void }) {
  const i = pages.findIndex((p) => p.path === current);
  const prev = pages[i - 1];
  const next = pages[i + 1];
  return <div className="mt-8 flex justify-between gap-2 border-t border-line-2 pt-3">
    {prev ? <button type="button" className="btn btn-sm" onClick={() => go(prev.path)}>‹ {prev.title}</button> : <span />}
    {next && <button type="button" className="btn btn-sm" onClick={() => go(next.path)}>{next.title} ›</button>}
  </div>;
}

// ---------------------------------------------------------------- Send feedback

/** Send feedback · N: the folio's bot thread first, then the sessions that wrote it, or New thread… with a bot. */
function SendFeedback({ ws, slug, unsent }: { ws: string; slug: string; unsent: FolioMark[] }) {
  const phone = usePhone();
  const [open, setOpen] = useState(false);
  const label = `Send feedback${unsent.length ? ` · ${unsent.length}` : ""}`;
  const button = (toggle: () => void, isOpen: boolean) => <button type="button" data-tour="folio.send" className={`btn btn-sm ${unsent.length ? "btn-primary" : ""}`} aria-expanded={isOpen} onClick={toggle} title={unsent.length ? `Send your ${unsent.length} unsent marks to the folio's agent` : "Nothing unsent: comment, decide or mark items first"}><Icon name="send" size={12} />{phone ? `Send${unsent.length ? ` · ${unsent.length}` : ""}` : label}</button>;
  if (phone) return <>
    {button(() => setOpen(true), open)}
    {open && <Sheet title={label} onClose={() => setOpen(false)}><Targets ws={ws} slug={slug} unsent={unsent} done={() => setOpen(false)} /></Sheet>}
  </>;
  return <Popover align="right" width={340} viewport trigger={({ open: o, toggle }) => button(toggle, o)}>
    {(close) => <Targets ws={ws} slug={slug} unsent={unsent} done={close} />}
  </Popover>;
}

type Sent = { session_id: string; bot_id: string | null; thread: string | null };

/** The toast's link to whoever got the message: the bot's thread, else the session. */
function sentTo(r: Sent, navigate: ReturnType<typeof useNavigate>) {
  return r.bot_id
    ? { label: "Open the thread", run: () => void navigate({ to: "/bots/$id", params: { id: r.bot_id! }, search: { thread: r.thread ?? undefined } as never }) }
    : { label: "Open the agent", run: () => void navigate({ to: "/agents", search: { session: r.session_id } as never }) };
}

function Targets({ ws, slug, unsent, done }: { ws: string; slug: string; unsent: FolioMark[]; done: () => void }) {
  const navigate = useNavigate();
  const targets = useFolioTargets(ws, slug);
  const bots = (useBots().data ?? []).filter((b) => b.workspace_id === ws);
  const [newThread, setNewThread] = useState(false);
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);
  const sendTo = (body: Record<string, unknown>) => {
    setBusy(true);
    void runAction<Sent>("folio.send-feedback", { workspace: ws, slug }, { ...body, note: note.trim() || undefined })
      .then((r) => { toast("Feedback sent", sentTo(r, navigate)); done(); })
      .catch(toastError).finally(() => setBusy(false));
  };
  const list = targets.data ?? [];
  return <div className="grid gap-2 p-3 text-[13px]">
    <p className="m-0 text-[12px] text-ink-3">Saved on Home. Nothing is sent until you choose where: {unsent.length ? marksSummary(unsent) : "nothing unsent yet"}.</p>
    <textarea className="field-input min-h-[56px] resize-y" placeholder="A line of your own (optional)" value={note} onChange={(e) => setNote(e.target.value)} />
    {targets.isLoading && <Loading what="recipients" />}
    {targets.error && <span className="text-[12px] text-danger">{humanError(targets.error)}</span>}
    {list.map((t, i) => <button key={`${t.kind}${t.session_id ?? t.thread_id}`} type="button" disabled={busy} className={`folio-target ${i === 0 ? "first" : ""}`} onClick={() => sendTo({ kind: t.kind, bot_id: t.bot_id ?? undefined, thread_id: t.thread_id ?? undefined, session_id: t.session_id ?? undefined })}>
      <span className="min-w-0 flex-1"><b className="block truncate">{t.label}</b><span className="block truncate text-[12px] text-ink-3">{t.detail}</span></span>
      {i === 0 && <span className="text-teal-ink"><Icon name="send" size={12} /></span>}
    </button>)}
    {!newThread
      ? <button type="button" className="folio-target" disabled={busy} onClick={() => setNewThread(true)}><span className="min-w-0 flex-1"><b className="block">New thread…</b><span className="block text-[12px] text-ink-3">A bot, with the agents of this folio as context</span></span><Icon name="plus" size={12} /></button>
      : <div className="grid gap-1 rounded-control border border-line p-2">
        <span className="text-[12px] text-ink-3">Start a new thread of:</span>
        {bots.map((b) => <button key={b.id} type="button" disabled={busy} className="folio-target" onClick={() => sendTo({ kind: "new_thread", bot_id: b.id })}><b className="min-w-0 flex-1 truncate">{b.settings.name}</b></button>)}
        {!bots.length && <span className="text-[12px] text-ink-3">This workspace has no bot.</span>}
      </div>}
  </div>;
}

// ---------------------------------------------------------------- side panels

function CommentsPanel({ d, ws, slug, draft, setDraft, readOnly, refresh, jump }: { d: FolioDetail; ws: string; slug: string; draft: Draft | null; setDraft: (d: Draft | null) => void; readOnly: boolean; refresh: () => void; jump: (m: { page: string | null; target: string | null }) => void }) {
  const [busy, setBusy] = useState(false);
  const comments = d.marks.filter((m) => m.kind === "comment");
  const replies = d.marks.filter((m) => m.kind === "reply");
  const unsent = unsentMarks(d.marks);
  const known = useMemo(() => new Set(d.pages.flatMap((p) => p.outline.items.flatMap((i) => (i.t === "heading" ? [i.id] : [])))), [d.pages]);
  const ids = useMemo(() => new Set(d.pages.flatMap((p) => [...p.text.matchAll(/\bid:\s*([A-Za-z0-9_.:-]+)/g)].map((m) => m[1]))), [d.pages]);
  const save = () => {
    if (!draft || !draft.text.trim()) return;
    setBusy(true);
    void runAction("folio.mark", { workspace: ws, slug }, { kind: "comment", page: draft.page, target: draft.target || undefined, quote: draft.quote, prefix: draft.prefix, suffix: draft.suffix, text: draft.text.trim() })
      .then(() => { setDraft(null); refresh(); }).catch(toastError).finally(() => setBusy(false));
  };
  const others = unsent.filter((m) => m.kind !== "comment");
  const open = comments.filter((c) => c.resolved_at == null);
  const resolved = comments.filter((c) => c.resolved_at != null);
  return <div className="grid gap-3 p-3 text-[13px]">
    {draft && <div className="grid gap-2 rounded-card border border-teal bg-paper p-2.5">
      <span className="text-[12px] text-ink-3">Comment on {draft.target ? <b>{draft.target}</b> : "this page"}{draft.quote ? <> · “{short(draft.quote, 60)}”</> : null}</span>
      <textarea autoFocus className="field-input min-h-[72px] resize-y" placeholder="Your comment (saved on Home; sent with Send feedback)" value={draft.text} onChange={(e) => setDraft({ ...draft, text: e.target.value })} />
      <div className="flex justify-end gap-2"><button type="button" className="btn btn-sm" onClick={() => setDraft(null)}>Cancel</button><button type="button" className="btn btn-sm btn-primary" disabled={busy || !draft.text.trim()} onClick={save}>Save</button></div>
    </div>}
    {!draft && !readOnly && <p className="m-0 text-[12px] text-ink-3">Select text, or use Comment on a section or an item. Each comment is saved at once; Send feedback sends them.</p>}
    {others.length > 0 && <div className="grid gap-1 rounded-card border border-line-2 bg-paper p-2.5">
      <b className="text-[12px]">Unsent marks</b>
      {(["verdict", "choice", "check"] as const).map((k) => { const ms = others.filter((m) => m.kind === k); return ms.length ? <span key={k} className="text-[12px] text-ink-2">{k === "verdict" ? "Verdicts" : k === "choice" ? "Decisions" : "Ticks"} · {ms.map((m) => `${m.target} ${k === "verdict" ? VERDICT_LABEL[m.value ?? ""] ?? m.value : k === "check" ? (m.value === "true" ? "✓" : "✗") : `→ ${m.value}`}`).join(" · ")}</span> : null; })}
    </div>}
    {open.map((c) => {
      const gone = !!c.target && !known.has(c.target) && !ids.has(c.target);
      // Its id is gone: the quote, found again, places it (else it shows as outdated, with its quote).
      const moved = gone ? relocate(d.pages, c) : null;
      return <CommentCard key={c.id} c={moved ? { ...c, ...moved } : c} replies={replies.filter((r) => r.parent === c.id)} ws={ws} slug={slug} outdated={gone && !moved} jump={jump} refresh={refresh} readOnly={readOnly} />;
    })}
    {!open.length && !draft && <Empty icon="comment">No open comments.</Empty>}
    {resolved.length > 0 && <details className="grid gap-2"><summary className="cursor-pointer text-[12px] text-ink-3">{resolved.length} resolved</summary>
      <div className="mt-2 grid gap-2">{resolved.map((c) => <CommentCard key={c.id} c={c} replies={replies.filter((r) => r.parent === c.id)} ws={ws} slug={slug} outdated={false} jump={jump} refresh={refresh} readOnly={readOnly} />)}</div>
    </details>}
  </div>;
}

/** Where a comment's quote is now (its page first): the page and the section around it, or null. */
export function relocate(pages: FolioDetail["pages"], c: Pick<FolioMark, "page" | "quote" | "prefix">): { page: string; target: string } | null {
  const flat = (x: string) => x.replace(/[*_`~]/g, "").replace(/\s+/g, " ").trim();
  const quote = flat(c.quote ?? "");
  if (quote.length < 3) return null;
  for (const p of [...pages].sort((a, b) => Number(b.path === c.page) - Number(a.path === c.page))) {
    const lines = p.text.split("\n");
    // The line where the quote starts (quotes may span lines: search the page with its line starts).
    let text = "";
    const starts: number[] = [];
    for (const l of lines) { starts.push(text.length); text += `${flat(l)} `; }
    const hits: number[] = [];
    for (let i = text.indexOf(quote); i >= 0; i = text.indexOf(quote, i + 1)) hits.push(i);
    if (!hits.length) continue;
    const prefix = flat(c.prefix ?? "");
    const at = hits.find((i) => prefix && text.slice(Math.max(0, i - prefix.length - 1), i).includes(prefix)) ?? hits[0];
    let line = 0;
    while (line + 1 < starts.length && starts[line + 1] <= at) line++;
    const heading = p.outline.items.filter((i) => i.t === "heading" && i.line <= line + 1).pop();
    return { page: p.path, target: heading && heading.t === "heading" ? heading.id : "" };
  }
  return null;
}

const short = (s: string, n: number) => (s.length > n ? `${s.slice(0, n - 1)}…` : s);

function CommentCard({ c, replies, ws, slug, outdated, jump, refresh, readOnly }: { c: FolioMark; replies: FolioMark[]; ws: string; slug: string; outdated: boolean; jump: (m: { page: string | null; target: string | null }) => void; refresh: () => void; readOnly: boolean }) {
  const state = c.resolved_at != null ? `resolved in rev ${c.resolved_rev}` : c.sent_at != null ? `sent ${ago(c.sent_at)}` : "not sent";
  return <div className={`grid gap-1.5 rounded-card border bg-paper p-2.5 ${c.sent_at == null ? "border-attn-line" : "border-line-2"}`}>
    <div className="flex items-center gap-2 text-[12px] text-ink-3">
      <span className="min-w-0 flex-1 truncate">You · {state}{outdated ? " · outdated" : ""}</span>
      {c.target && <button type="button" className="text-teal-ink hover:underline" onClick={() => jump(c)} title="Show it in the page">{c.page && c.page !== "index.md" ? `${c.page} › ` : ""}{c.target}</button>}
    </div>
    {c.quote && <blockquote className="m-0 border-l-2 border-line pl-2 text-[12px] text-ink-2">{short(c.quote, 160)}</blockquote>}
    <p className="m-0 whitespace-pre-wrap">{c.text}</p>
    {replies.map((r) => <div key={r.id} className="grid gap-0.5 rounded-control bg-sunk px-2 py-1.5"><span className="text-[12px] text-ink-3">{r.author.name} · {ago(r.created_at)}</span><span className="whitespace-pre-wrap">{r.text}</span></div>)}
    {!readOnly && <div className="flex justify-end gap-1.5">
      {c.sent_at == null && <ConfirmButton confirmLabel="Delete" consequence="Deletes this comment." onConfirm={() => runAction("folio.unmark", { workspace: ws, slug, id: String(c.id) }).then(refresh)}>Delete</ConfirmButton>}
      {c.sent_at != null && <button type="button" className="btn btn-sm" onClick={() => void runAction("folio.resolve", { workspace: ws, slug, id: String(c.id) }, c.resolved_at != null ? { open: true } : {}).then(refresh).catch(toastError)}>{c.resolved_at != null ? "Reopen" : "Resolve"}</button>}
    </div>}
  </div>;
}

function VersionsPanel({ ws, slug, current, latest, view }: { ws: string; slug: string; current: number | null; latest: number; view: (rev: number | null) => void }) {
  const revs = useFolioRevs(ws, slug);
  const [shown, setShown] = useState<number | null>(null);
  const diff = useFolioDiff(ws, slug, shown);
  if (revs.isLoading) return <Loading what="versions" />;
  if (revs.error) return <ErrorState error={revs.error} />;
  return <div className="grid gap-2 p-3 text-[13px]">
    <p className="m-0 text-[12px] text-ink-3">Every write is a rev, kept 30 days with its author. Restore puts a rev's files back as a new rev.</p>
    {(revs.data ?? []).map((r) => <div key={r.rev} className={`grid gap-1 rounded-card border bg-paper p-2.5 ${current === r.rev || (current == null && r.rev === latest) ? "border-teal" : "border-line-2"}`}>
      <div className="flex items-center gap-2">
        <b className="text-[13px]">rev {r.rev}</b>
        <span className="min-w-0 flex-1 truncate text-[12px] text-ink-3" title={when(r.at)}>{r.author.name} · {ago(r.at)}{r.label ? ` · ${r.label}` : ""}</span>
        <MoreMenu label={`Rev ${r.rev} actions`} width={220}>{(close) => <>
          <MenuItem icon={<Icon name="eye" size={14} />} label={r.rev === latest ? "View the latest" : "View this version"} onClick={() => { close(); view(r.rev === latest ? null : r.rev); }} />
          <MenuItem icon={<Icon name="file" size={14} />} label={shown === r.rev ? "Hide the diff" : "Diff"} onClick={() => { close(); setShown(shown === r.rev ? null : r.rev); }} />
          <MenuSection><ConfirmButton className="w-full" confirmLabel="Restore" consequence="Puts the files of this rev back as they were before it, as a new rev." onConfirm={() => runAction("folio.restore", { workspace: ws, slug, rev: String(r.rev) }).then(() => { close(); toast(`Restored the files from before rev ${r.rev}`); })}>Restore before this rev</ConfirmButton></MenuSection>
        </>}</MoreMenu>
      </div>
      <span className="text-[12px] text-ink-2">{r.files.map((f) => `${f.path}${f.added != null ? ` (+${f.added} −${f.removed})` : ""}`).join(", ")}{r.errors ? ` · ${r.errors} error${r.errors === 1 ? "" : "s"}` : ""}</span>
      {shown === r.rev && (diff.isLoading ? <Loading what="diff" /> : diff.data?.files.map((f) => <div key={f.path} className="grid gap-1"><span className="text-[12px] font-semibold">{f.path}</span>{f.patch ? <Patch text={f.patch} /> : <span className="text-[12px] text-ink-3">{f.created ? "Added" : f.deleted ? "Removed" : "Changed"} (not text)</span>}</div>))}
    </div>)}
  </div>;
}

function Patch({ text }: { text: string }) {
  return <pre className="max-h-[320px] overflow-auto rounded-control border border-line-2 bg-code p-1.5 font-mono text-[12px] leading-[1.45]">{text.split("\n").map((l, i) => <div key={i} className={l.startsWith("+") ? "bg-diff-add" : l.startsWith("-") ? "bg-diff-del" : l.startsWith("@@") ? "text-ink-3" : ""}>{l || " "}</div>)}</pre>;
}

function ProblemsPanel({ d, ws, slug, readOnly, jump }: { d: FolioDetail; ws: string; slug: string; readOnly: boolean; jump: (file: string, line: number | null) => void }) {
  const navigate = useNavigate();
  const targets = useFolioTargets(ws, slug, d.problems.length > 0 && !readOnly);
  const [busy, setBusy] = useState(false);
  if (!d.problems.length) return <Empty icon="check">No problems in rev {d.rev}.</Empty>;
  const tone = (s: string) => (s === "error" ? "text-danger" : s === "warn" ? "text-attn" : "text-ink-3");
  // The folio's thread first, else the last session that wrote it: the same order as Send feedback.
  const to = targets.data?.[0];
  const ask = () => {
    if (!to) return;
    setBusy(true);
    void runAction<Sent>("folio.ask-fix", { workspace: ws, slug }, { kind: to.kind, bot_id: to.bot_id ?? undefined, thread_id: to.thread_id ?? undefined, session_id: to.session_id ?? undefined })
      .then((r) => toast(`Asked ${to.label} to fix the problems`, sentTo(r, navigate))).catch(toastError).finally(() => setBusy(false));
  };
  return <div className="grid gap-2 p-3 text-[13px]">
    <p className="m-0 text-[12px] text-ink-3">Home's check of rev {d.rev}. The agent sees the same list after each write and fixes it with launchpad folio check --fix or a patch.</p>
    {!readOnly && <div className="flex items-center gap-2">
      <button type="button" className="btn btn-sm" disabled={busy || !to} onClick={ask} title={to ? `Sends one line to ${to.label} (${to.detail}); your marks stay unsent` : "No agent to ask: no bot thread or session wrote this folio"}><Icon name="send" size={12} />Ask the author to fix</button>
      {to && <span className="min-w-0 truncate text-[12px] text-ink-3">{to.label}</span>}
    </div>}
    {d.problems.map((p, i) => <button key={i} type="button" className="grid gap-0.5 rounded-card border border-line-2 bg-paper p-2.5 text-left" onClick={() => jump(p.file, p.line)}>
      <span className={`text-[12px] font-semibold ${tone(p.severity)}`}>{p.severity === "error" ? "Error" : p.severity === "warn" ? "Warning" : "Hint"} · {p.code}</span>
      <span className="text-[12px] text-ink-3">{p.file}{p.line ? `:${p.line}` : ""}</span>
      <span>{p.message}</span>
      {p.hint && <span className="text-[12px] text-ink-2">{p.hint}</span>}
    </button>)}
  </div>;
}
