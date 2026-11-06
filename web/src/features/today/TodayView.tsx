// Today (laptop home `/` and the phone's first tab): every decision of the window's workspace, in the order to make
// it, from the decision list Home computes (crates/core/src/today.rs), one compact row each (TodayRows.tsx). Each row carries its main action from the
// action list: Answer, Merge (only when safe), Start, Send, Post. Filters are chips with their own address
// (`/?filter=send`), and `/?card=<item id>` opens one row. Done lists what was handled in the last 30 days.
// Nothing reaches GitHub or other people without the card's button and its confirmation.

import { useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate, useSearch } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { Proposal } from "@/gen/Proposal";
import { Icon } from "@/components/Icon";
import { toastError } from "@/components/Toast";
import { ConfirmButton, Empty, ErrorState, FilterChip, FilterChips, Loading, PageHeader } from "@/components/ui";
import { runProposalAction, useProposalList } from "@/features/proposals/ProposalCard";
import { summary } from "@/features/proposals/ProposalList";
import { useSuggestionList } from "@/features/suggestions/data";
import { ago } from "@/lib/format";
import { usePhone } from "@/lib/phone";
import { useScope, useWorkspaces } from "@/lib/workspace";
import { useWorkspace } from "@/stores/workspace";
import { FILTERS, isView, useTodayHere, type TodayItem, type TodayView as View } from "./data";
import { ItemRow, LOOK } from "./TodayRows";

/** Handled in the last 30 days: what it was, what happened and when. */
const DoneRow = memo(function DoneRow({ item }: { item: TodayItem }) {
  return <div className="grid h-[52px] grid-cols-[28px_minmax(0,1fr)_auto] items-center gap-x-3 border-b border-line-2 px-4">
    <span className="grid size-7 place-items-center text-ink-3" title={LOOK[item.kind].name}><Icon name={LOOK[item.kind].icon} size={15} /></span>
    <div className="min-w-0">
      <div className="truncate text-[13px] font-medium" title={item.title}>{item.title}</div>
      <div className="truncate text-[12px] text-ink-3">{item.outcome} · {ago(item.at)}{item.detail ? ` · ${item.detail}` : ""}</div>
    </div>
    {item.session_id ? <Link className="btn btn-sm no-underline" to="/agents" search={{ session: item.session_id }}>Open agent</Link> : <span />}
  </div>;
});

const EMPTY: Record<View, string> = {
  all: "Nothing waits for you. Questions, finished agents, proposals, drafts and reviews show up here.",
  answer: "No agent waits for an answer.",
  review: "Nothing to review: no unseen finished agent and no Local PR to publish.",
  approve: "No proposal waits for you.",
  send: "No message waits to be sent.",
  github: "No suggestion to post and no review request.",
  fix: "No failed routine and no runner to update.",
  done: "Nothing handled in the last 30 days.",
};

/** Start every waiting launch card of the list, with one confirmation. */
function StartAll({ proposals }: { proposals: Proposal[] }) {
  const qc = useQueryClient();
  const [busy, setBusy] = useState(false);
  const startable = proposals.filter((p) => p.state === "proposed" && p.body.kind !== "status" && p.body.kind !== "ready");
  if (startable.length < 2) return null;
  const run = async () => {
    setBusy(true);
    try {
      for (const p of startable) {
        try { await runProposalAction(p.id, "accept"); } catch (e) { toastError(e); }
      }
      await Promise.all([qc.invalidateQueries({ queryKey: ["proposals"] }), qc.invalidateQueries({ queryKey: ["sessions"] })]);
    } finally { setBusy(false); }
  };
  return <ConfirmButton tone="publish" className="btn-primary" icon="play" disabled={busy} consequence={`Start ${summary(startable)}?`} confirmLabel="Start all" onConfirm={run}>Start all ({startable.length})</ConfirmButton>;
}

type Entry = { item?: TodayItem; heading?: TodayItem["filter"] };
const LABEL = Object.fromEntries(FILTERS.map(([f, label]) => [f, label])) as Record<TodayItem["filter"], string>;

export function TodayView() {
  const phone = usePhone();
  const scope = useScope();
  const navigate = useNavigate();
  const search = useSearch({ strict: false }) as { filter?: string; card?: string };
  const { items, done, loading, error } = useTodayHere();
  const proposals = useProposalList().data;
  const suggestions = useSuggestionList().data;
  const byProposal = useMemo(() => new Map((proposals ?? []).map((p) => [p.id, p])), [proposals]);
  const bySuggestion = useMemo(() => new Map((suggestions ?? []).map((s) => [s.id, s])), [suggestions]);
  // A card link (`?card=`) shows every waiting card, or Done when the card was handled.
  const cardDone = !!search.card && !items.some((i) => i.id === search.card) && done.some((i) => i.id === search.card);
  const view: View = search.card ? (cardDone ? "done" : "all") : isView(search.filter) ? search.filter : "all";
  const setView = (v: View) => void navigate({ to: "/", search: v === "all" ? {} : { filter: v }, replace: true });
  const shown = view === "all" ? items : view === "done" ? done : items.filter((i) => i.filter === view);
  const counts = useMemo(() => Object.fromEntries(FILTERS.map(([f]) => [f, items.filter((i) => i.filter === f).length])), [items]);
  const waitingProposals = useMemo(() => items.filter((i) => i.kind === "proposal").map((i) => byProposal.get(i.record)).filter((p): p is Proposal => !!p), [items, byProposal]);

  // All: a heading before each kind of decision (the list comes sorted by filter).
  const rows = useMemo<Entry[]>(() => {
    if (view !== "all") return shown.map((item) => ({ item }));
    const out: Entry[] = [];
    for (const item of shown) {
      if (out.at(-1)?.item?.filter !== item.filter) out.push({ heading: item.filter });
      out.push({ item });
    }
    return out;
  }, [shown, view]);
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(() => new Set(search.card ? [search.card] : []));
  const toggle = useCallback((id: string) => setExpanded((old) => { const next = new Set(old); if (!next.delete(id)) next.add(id); return next; }), []);

  const ref = useRef<HTMLDivElement>(null);
  const list = useVirtualizer({
    count: rows.length,
    getScrollElement: () => ref.current,
    estimateSize: (i) => (rows[i].heading ? 36 : phone ? 61 : 54),
    getItemKey: (i) => rows[i].item?.id ?? `heading:${rows[i].heading}`,
    overscan: 6,
    useFlushSync: false,
  });
  // Open the linked card once the list has it.
  const revealed = useRef<string | null>(null);
  useEffect(() => {
    if (!search.card || revealed.current === search.card) return;
    const index = rows.findIndex((r) => r.item?.id === search.card);
    if (index < 0) return;
    revealed.current = search.card;
    setExpanded((old) => new Set(old).add(search.card!));
    list.scrollToIndex(index, { align: "start" });
  }, [search.card, rows, list]);
  // A session's card that no longer waits and was not handled here (seen elsewhere, still running): open the session.
  // Only once the window shows its workspace: a link's `?ws=` switches it after the workspaces load.
  const current = useWorkspace((st) => (st.requested === null ? st.current : null));
  const workspaces = useWorkspaces().data;
  const settled = !!current && !!workspaces?.some((w) => w.id === current);
  useEffect(() => {
    if (loading || error || !settled || !search.card?.startsWith("session:") || shown.some((i) => i.id === search.card)) return;
    void navigate({ to: "/agents", search: { session: search.card.slice("session:".length) }, replace: true });
  }, [loading, error, settled, search.card, shown, navigate]);

  const chips = <FilterChips tour="today.filters" label="Today filters">
    <FilterChip active={view === "all"} count={items.length} onClick={() => setView("all")}>All</FilterChip>
    {FILTERS.map(([id, label, title]) => (counts[id] > 0 || view === id) && <FilterChip key={id} active={view === id} count={counts[id]} title={title} onClick={() => setView(id)}>{label}</FilterChip>)}
    <FilterChip active={view === "done"} title="What you handled in the last 30 days, with where it went" onClick={() => setView("done")}>Done</FilterChip>
  </FilterChips>;
  const groupActions = (view === "approve" || view === "all") && <StartAll proposals={waitingProposals} />;
  const meta = `${items.length ? `${items.length} decision${items.length === 1 ? "" : "s"}` : "Nothing waits"} in ${scope.workspace.name}`;

  return (
    <div className="flex h-full min-h-0 flex-col" data-tour="today.page">
      {phone
        ? <div className="phone-page-content grid gap-1 border-b border-line-2 py-1.5"><div className="flex flex-wrap items-center justify-between gap-2"><span className="truncate text-xs text-ink-3">{meta}</span>{groupActions}</div>{chips}</div>
        : <PageHeader title="Today" meta={<span>{meta}</span>} intro="Everything that waits for your decision, one row each, grouped by what to do. Hover an icon to see what it is; click a title to read more. Nothing reaches GitHub or other people without your click." actions={groupActions || undefined} tabs={<div className="pb-3">{chips}</div>} />}
      <div ref={ref} className="min-h-0 flex-1 overflow-y-auto [overflow-wrap:anywhere]">
        {error ? <ErrorState className="m-4" error={error} /> : loading ? <Loading what="Today" /> : shown.length === 0 && <Empty fill={phone} icon="check" className={phone ? "" : "mx-auto max-w-[760px] pt-6"}>{EMPTY[view]}</Empty>}
        <div className={`relative mx-auto w-full ${view === "done" ? "max-w-[860px]" : "max-w-[1040px]"}`} style={{ height: list.getTotalSize() }}>
          {list.getVirtualItems().map((row) => {
            const { item, heading } = rows[row.index];
            return (
              <div key={row.key} data-index={row.index} ref={list.measureElement} className="absolute top-0 left-0 w-full" style={{ transform: `translateY(${row.start}px)` }}>
                {heading ? <h2 className="m-0 flex items-baseline gap-2 border-b border-line-2 px-3 pt-4 pb-1.5 text-[11px] font-semibold tracking-wide text-ink-3 uppercase">{LABEL[heading]}<span className="font-normal normal-case">{counts[heading]}</span></h2>
                  : view === "done" ? <DoneRow item={item!} />
                  : <ItemRow item={item!} expanded={expanded.has(item!.id)} toggle={toggle} highlight={item!.id === search.card} proposal={byProposal.get(item!.record)} suggestion={bySuggestion.get(item!.record)} />}
              </div>
            );
          })}
        </div>
        <div className="h-4" />
      </div>
    </div>
  );
}
