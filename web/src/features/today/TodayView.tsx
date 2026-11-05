// Today (laptop home `/` and the phone's first tab): every decision of the window's workspace, in the order to make
// it, from the decision list Home computes (crates/core/src/today.rs). Each card carries its main action from the
// action list: Answer, Merge (only when safe), Start, Send, Post. Filters are chips with their own address
// (`/?filter=send`), and `/?card=<item id>` opens one card. Done lists what was handled in the last 30 days.
// Nothing reaches GitHub or other people without the card's button and its confirmation.

import { useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate, useSearch } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useEffect, useMemo, useRef, useState } from "react";
import type { Proposal } from "@/gen/Proposal";
import { Icon } from "@/components/Icon";
import { toastError } from "@/components/Toast";
import { ConfirmButton, Empty, ErrorState, FilterChip, FilterChips, Loading, PageHeader } from "@/components/ui";
import { runProposalAction, useProposalList } from "@/features/proposals/ProposalCard";
import { summary } from "@/features/proposals/ProposalList";
import { useSuggestionList } from "@/features/suggestions/data";
import { formatAge } from "@/lib/format";
import { usePhone } from "@/lib/phone";
import { useScope } from "@/lib/workspace";
import { BotField } from "./AskField";
import { FILTERS, isView, useTodayHere, type TodayItem, type TodayView as View } from "./data";
import { ICON, ItemCard } from "./TodayCards";

/** Handled in the last 30 days: what it was, what happened and when. */
const DoneRow = memo(function DoneRow({ item }: { item: TodayItem }) {
  return <div className="grid h-[52px] grid-cols-[28px_minmax(0,1fr)_auto] items-center gap-x-3 border-b border-line-2 px-4">
    <span className="grid size-7 place-items-center rounded-lg bg-sunk text-ink-3"><Icon name={ICON[item.kind]} size={13} /></span>
    <div className="min-w-0">
      <div className="truncate text-[13px] font-medium" title={item.title}>{item.title}</div>
      <div className="truncate text-[12px] text-ink-3">{item.outcome} · {formatAge(item.at)} ago{item.detail ? ` · ${item.detail}` : ""}</div>
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

  const ref = useRef<HTMLDivElement>(null);
  const list = useVirtualizer({
    count: shown.length,
    getScrollElement: () => ref.current,
    estimateSize: () => (view === "done" ? 52 : 160),
    getItemKey: (i) => shown[i].id,
    overscan: 3,
    useFlushSync: false,
  });
  // Open the linked card once the list has it.
  const revealed = useRef<string | null>(null);
  useEffect(() => {
    if (!search.card || revealed.current === search.card) return;
    const index = shown.findIndex((i) => i.id === search.card);
    if (index < 0) return;
    revealed.current = search.card;
    list.scrollToIndex(index, { align: "start" });
  }, [search.card, shown, list]);

  const chips = <FilterChips tour="today.filters" label="Today filters">
    <FilterChip active={view === "all"} count={items.length} onClick={() => setView("all")}>All</FilterChip>
    {FILTERS.map(([id, label, title]) => (counts[id] > 0 || view === id) && <FilterChip key={id} active={view === id} count={counts[id]} title={title} onClick={() => setView(id)}>{label}</FilterChip>)}
    <FilterChip active={view === "done"} title="What you handled in the last 30 days, with where it went" onClick={() => setView("done")}>Done</FilterChip>
  </FilterChips>;
  const groupActions = (view === "approve" || view === "all") && <StartAll proposals={waitingProposals} />;
  const meta = `${items.length ? `${items.length} decision${items.length === 1 ? "" : "s"}` : "Nothing waits"} in ${scope.workspace.name} · nothing reaches GitHub or other people without your click`;

  return (
    <div className="flex h-full min-h-0 flex-col" data-tour="today.page">
      {phone
        ? <div className="phone-page-content grid gap-1.5 border-b border-line-2 py-2"><span className="text-xs text-ink-3">{meta}</span>{chips}{groupActions && <div>{groupActions}</div>}</div>
        : <PageHeader title="Today" meta={<span>{meta}</span>} actions={groupActions || undefined} tabs={<div className="pb-3">{chips}</div>} />}
      {!phone && <div className="mx-auto w-full max-w-[760px] px-4 pt-3"><BotField /></div>}
      <div ref={ref} className="min-h-0 flex-1 overflow-y-auto [overflow-wrap:anywhere]">
        {error ? <ErrorState className="m-4" error={error} /> : loading ? <Loading what="Today" /> : shown.length === 0 && <Empty fill={phone} icon="check" className={phone ? "" : "mx-auto max-w-[760px] pt-6"}>{EMPTY[view]}</Empty>}
        <div className={`relative mx-auto w-full ${view === "done" ? "max-w-[860px]" : "max-w-[760px]"}`} style={{ height: list.getTotalSize() }}>
          {list.getVirtualItems().map((row) => {
            const item = shown[row.index];
            return (
              <div key={row.key} data-index={row.index} ref={list.measureElement} className={`absolute top-0 left-0 w-full ${view === "done" ? "" : "px-3 pt-3 sm:px-4"}`} style={{ transform: `translateY(${row.start}px)` }}>
                {view === "done" ? <DoneRow item={item} /> : <ItemCard item={item} proposal={byProposal.get(item.record)} suggestion={bySuggestion.get(item.record)} />}
              </div>
            );
          })}
        </div>
        <div className="h-4" />
      </div>
    </div>
  );
}
