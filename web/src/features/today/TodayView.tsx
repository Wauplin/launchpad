// Today (laptop home `/` and the phone's first tab): every decision of the window's workspace, in the order to make
// it, from the decision list Home computes (crates/core/src/today.rs). Each card carries its main action from the
// action list: Answer, Merge (only when safe), Start, Send, Post. Filters are chips with their own address
// (`/?filter=send`), and `/?card=<item id>` opens one card. Done lists what was handled in the last 30 days.
// Nothing reaches GitHub or other people without the card's button and its confirmation.

import { useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate, useSearch } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import type { Proposal } from "@/gen/Proposal";
import type { Session } from "@/gen/Session";
import type { Suggestion } from "@/gen/Suggestion";
import type { WorktreeInfo } from "@/gen/WorktreeInfo";
import { ActionButton } from "@/components/ActionButton";
import { HarnessMark } from "@/components/HarnessMark";
import { Icon, type IconName } from "@/components/Icon";
import { toastError } from "@/components/Toast";
import { ConfirmButton, Empty, ErrorState, FilterChip, FilterChips, Loading, PageHeader } from "@/components/ui";
import { AskCard } from "@/features/agents/AskCard";
import { buildRows } from "@/features/agents/rows";
import { StatusBadge } from "@/features/agents/SessionBits";
import { useSessionWorktrees, worktreeSession } from "@/features/agents/Worktrees";
import { reviewWithAgent } from "@/features/github/reviewAgent";
import { useInbox } from "@/features/inbox/data";
import { MessageCard, useOutboxMessages } from "@/features/outbox/MessageCard";
import { ProposalCard, runProposalAction, useProposalList } from "@/features/proposals/ProposalCard";
import { summary } from "@/features/proposals/ProposalList";
import { SuggestionCard } from "@/features/suggestions/Suggestions";
import { useSuggestionList } from "@/features/suggestions/data";
import { runAction } from "@/lib/actions";
import { formatAge } from "@/lib/format";
import { usePhone } from "@/lib/phone";
import { projectName, useProjects, useSession } from "@/lib/queries";
import { emptyLog, useSessionLogs } from "@/lib/sessionStore";
import { isDone } from "@/lib/sessionStatus";
import { useScope } from "@/lib/workspace";
import { BotField } from "./AskField";
import { FILTERS, isView, useTodayHere, type TodayItem, type TodayView as View } from "./data";

const ICON: Record<TodayItem["kind"], IconName> = {
  question: "alert", finished: "check", failed: "xcircle", local_pr: "pr-local", proposal: "spark", outbox: "send", suggestion: "comment", review_request: "pr", routine: "refresh", runner: "window",
};

const ghParams = (repo: string, number: number) => ({ owner: repo.split("/")[0], repo: repo.split("/")[1], number: String(number) });

/** The card frame: icon, title, one meta line, then the kind's own content and buttons. `age: false` when the meta already says since when (a session's status badge). */
function Card({ item, meta, open, children, tone = "border-line", age = true }: { item: TodayItem; meta?: ReactNode; open?: ReactNode; children?: ReactNode; tone?: string; age?: boolean }) {
  return (
    <article data-tour="today.card" data-tour-id={item.id} className={`grid min-w-0 grid-cols-1 gap-2.5 rounded-xl border bg-paper p-3 ${tone}`}>
      <div className="flex min-w-0 items-start gap-2.5">
        <span className="grid size-7 flex-none place-items-center rounded-lg bg-sunk text-ink-2"><Icon name={ICON[item.kind]} size={14} /></span>
        <div className="min-w-0 flex-1">
          <div className="line-clamp-2 text-[13px] font-semibold" title={item.title}>{item.title}</div>
          <div className="flex min-w-0 flex-wrap items-center gap-x-1.5 text-[12px] text-ink-3">{meta ?? <span className="truncate">{item.detail}</span>}{age && <span className="flex-none">· {formatAge(item.at)} ago</span>}</div>
        </div>
        {open && <span className="flex-none">{open}</span>}
      </div>
      {children}
    </article>
  );
}

const OpenSession = ({ id, view }: { id: string; view?: "changes" }) => <Link className="btn btn-sm no-underline" to="/agents" search={{ session: id, ...(view ? { view } : {}) }}>Open</Link>;

function SessionMeta({ s, item }: { s: Session | null; item: TodayItem }) {
  const projects = useProjects().data;
  if (!s) return <span className="truncate">{item.detail}</span>;
  return <><HarnessMark harness={s.harness} size={13} /><span className="truncate">{projectName(projects, s.project_id, s.cwd)}</span><StatusBadge s={s} /></>;
}

/** Loads the newest events of a session (the same tail page the session view uses). */
function useTail(id: string) {
  const log = useSessionLogs((st) => st.logs[id]);
  const load = useSessionLogs((st) => st.load);
  useEffect(() => {
    if (log) return;
    useSessionLogs.setState((st) => ({ logs: { ...st.logs, [id]: emptyLog() } }));
    void load(id);
  }, [id, log, load]);
  return log;
}

/** Answer in place: the open question's options, as in the session. */
function QuestionCard({ item }: { item: TodayItem }) {
  const s = useSession(item.record);
  const log = useTail(item.record);
  const ask = useMemo(() => {
    const rows = log ? buildRows(log) : [];
    for (let i = rows.length - 1; i >= 0; i--) { const r = rows[i]; if (r.t === "ask" && !r.answers) return r; }
    return null;
  }, [log]);
  return <Card item={item} tone="border-attn-line" age={!s} meta={<SessionMeta s={s} item={item} />} open={<OpenSession id={item.record} />}>
    {ask && s?.live ? <AskCard row={ask} sessionId={item.record} live={s.live} hint="Or open the agent to type another answer." /> : <p className="m-0 text-[13px] text-ink-2">{item.detail || "The agent asked a question."}</p>}
  </Card>;
}

/** A finished agent: Merge from the card only when it is safe (Done, its runner here, branch on Home, nothing uncommitted, base unchanged); otherwise Review changes. */
function FinishedCard({ item }: { item: TodayItem }) {
  const qc = useQueryClient();
  const s = useSession(item.record);
  const worktrees = useSessionWorktrees(item.record).data;
  const w = worktrees?.find((i) => i.worktree.primary) ?? worktrees?.[0];
  const t = w?.worktree;
  const base = t?.base_branch;
  const safe = !!s && isDone(s) && !!w && w.exists && !w.away && !!t && !!base && t.state === "active" && !t.merge && !t.pull_request && !t.predraft_id && w.ahead > 0 && w.dirty_files === 0 && w.behind === 0 && !w.unpushed && !t.push_note;
  const facts = w && t ? [
    w.ahead ? `${w.ahead} commit${w.ahead === 1 ? "" : "s"} ahead of ${base ?? "base"}` : t.merged ? `merged into ${t.merged.into}` : "no new commit",
    w.dirty_files ? `${w.dirty_files} uncommitted` : "",
    w.behind ? `${base} moved` : "",
    w.unpushed || t.push_note ? "not on Home yet" : "",
  ].filter(Boolean).join(" · ") : null;
  // Merged: the card is handled. A merge that waits or stopped keeps the card, with Review changes.
  const merged = async (r: WorktreeInfo) => {
    void qc.invalidateQueries({ queryKey: ["worktrees"] });
    if (r.worktree.merged) await runAction("session.seen", { id: item.record }).catch(toastError);
  };
  return <Card item={item} age={!s} meta={<SessionMeta s={s} item={item} />} open={<OpenSession id={item.record} />}>
    {s?.last_message && <p className="m-0 line-clamp-3 text-[13px] text-ink-2">{s.last_message}</p>}
    {facts && <div className="flex min-w-0 items-center gap-1.5 text-[12px] text-ink-2"><Icon name="branch" size={13} className="flex-none text-ink-3" /><span className="truncate font-mono">{t!.branch}</span><span className="truncate">· {facts}</span></div>}
    <div className="flex flex-wrap items-center gap-1.5">
      {safe
        ? <ActionButton name="worktree.merge" params={{ id: t!.id }} vars={{ branch: t!.branch, base: base! }} query={{ session: worktreeSession(t!) ?? item.record }} primary onDone={merged}>Merge into {base}</ActionButton>
        : w && w.ahead > 0 && <Link className="btn btn-sm btn-primary no-underline" to="/agents" search={{ session: item.record, view: "changes" }}>Review changes</Link>}
      <ActionButton name="session.archive" params={{ id: item.record }} />
      <ActionButton name="session.seen" params={{ id: item.record }} title="Keep the agent and remove the card" />
    </div>
  </Card>;
}

function FailedCard({ item }: { item: TodayItem }) {
  const s = useSession(item.record);
  return <Card item={item} tone="border-gh-closed/40" age={!s} meta={<SessionMeta s={s} item={item} />} open={<OpenSession id={item.record} />}>
    {s?.last_message && <p className="m-0 line-clamp-3 text-[13px] text-ink-2">{s.last_message}</p>}
    <div className="flex flex-wrap items-center gap-1.5">
      <ActionButton name="session.resume" params={{ id: item.record }} body={{ continue_turn: true }} primary />
      <ActionButton name="session.archive" params={{ id: item.record }} />
      <ActionButton name="session.seen" params={{ id: item.record }} title="Keep the agent and remove the card" />
    </div>
  </Card>;
}

function LocalPrCard({ item }: { item: TodayItem }) {
  return <Card item={item} open={item.session_id ? <OpenSession id={item.session_id} /> : undefined}>
    <div className="flex flex-wrap items-center gap-1.5"><Link className="btn btn-sm btn-primary no-underline" to="/predrafts/$id" params={{ id: item.record }}>Review &amp; publish</Link><span className="text-[12px] text-ink-3">Publish and Merge locally are on its page.</span></div>
  </Card>;
}

function ProposalItem({ item, p }: { item: TodayItem; p: Proposal | undefined }) {
  if (!p) return <Card item={item}><Loading what="the card" /></Card>;
  return <ProposalCard p={p} />;
}

function OutboxItem({ item }: { item: TodayItem }) {
  const m = useOutboxMessages().data?.find((x) => x.id === item.record);
  if (!m) return <Card item={item}><Loading what="the message" /></Card>;
  return <MessageCard m={m} />;
}

function SuggestionItem({ item, s }: { item: TodayItem; s: Suggestion | undefined }) {
  if (!s || s.target.kind !== "git_hub") return <Card item={item}><Loading what="the suggestion" /></Card>;
  const t = s.target;
  return <div className="grid gap-1.5">
    <div className="flex flex-wrap items-center gap-2 px-1 text-[12px] text-ink-3">
      <Icon name="pr" size={12} />
      <Link to="/github/$owner/$repo/$number" params={ghParams(t.repo, t.number)} search={{ draft: s.id }} className="text-teal-ink no-underline hover:underline">{t.repo}#{t.number}</Link>
      <span>· {formatAge(s.created_at)} ago</span>
      {s.session_id && <Link to="/agents" search={{ session: s.session_id }} className="text-teal-ink no-underline hover:underline">Open agent</Link>}
    </div>
    <SuggestionCard suggestion={s} />
  </div>;
}

function ReviewRequestCard({ item }: { item: TodayItem }) {
  const projects = useProjects().data ?? [];
  const t = useInbox().data?.threads.find((x) => x.id === item.record);
  if (!t || t.number === null) return <Card item={item} />;
  const n = t.number;
  return <Card item={item} open={<Link className="btn btn-sm no-underline" to="/github/$owner/$repo/$number" params={ghParams(t.repo, n)}>Open PR</Link>}>
    <div className="flex flex-wrap items-center gap-1.5">
      <button className="btn btn-sm btn-primary" title="Opens a launch draft that asks an agent for a local suggested review" onClick={() => reviewWithAgent({ projects, repo: t.repo, number: n, title: t.title, url: t.url })}><Icon name="spark" size={12} />Review with agent</button>
      <span className="text-[12px] text-ink-3">The review comes back here as a suggestion to post.</span>
    </div>
  </Card>;
}

function RoutineCard({ item }: { item: TodayItem }) {
  return <Card item={item} open={item.session_id ? <OpenSession id={item.session_id} /> : undefined}>
    <div><Link className="btn btn-sm btn-primary no-underline" to="/schedules" search={{ routine: item.record }}>Open routine</Link></div>
  </Card>;
}

function RunnerCard({ item }: { item: TodayItem }) {
  return <Card item={item}><div><Link className="btn btn-sm btn-primary no-underline" to="/settings/runners">Open runners</Link></div></Card>;
}

const ItemCard = memo(function ItemCard({ item, proposal, suggestion }: { item: TodayItem; proposal: Proposal | undefined; suggestion: Suggestion | undefined }) {
  switch (item.kind) {
    case "question": return <QuestionCard item={item} />;
    case "finished": return <FinishedCard item={item} />;
    case "failed": return <FailedCard item={item} />;
    case "local_pr": return <LocalPrCard item={item} />;
    case "proposal": return <ProposalItem item={item} p={proposal} />;
    case "outbox": return <OutboxItem item={item} />;
    case "suggestion": return <SuggestionItem item={item} s={suggestion} />;
    case "review_request": return <ReviewRequestCard item={item} />;
    case "routine": return <RoutineCard item={item} />;
    case "runner": return <RunnerCard item={item} />;
  }
});

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
  const startable = proposals.filter((p) => p.state === "proposed" && p.body.kind !== "status");
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
