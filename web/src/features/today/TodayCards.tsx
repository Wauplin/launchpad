// The cards of Today, one per kind of decision, each with its main action from the action list. The bot chat
// shows the same cards for what a bot's session wrote (ready clicks) and the same branch actions (Changes).

import { useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { memo, useEffect, useMemo, type ReactNode } from "react";
import type { Proposal } from "@/gen/Proposal";
import type { Session } from "@/gen/Session";
import type { Suggestion } from "@/gen/Suggestion";
import type { WorktreeInfo } from "@/gen/WorktreeInfo";
import { ActionButton } from "@/components/ActionButton";
import { HarnessMark } from "@/components/HarnessMark";
import { Icon, type IconName } from "@/components/Icon";
import { toastError } from "@/components/Toast";
import { Loading } from "@/components/ui";
import { AskCard } from "@/features/agents/AskCard";
import { buildRows } from "@/features/agents/rows";
import { StatusBadge } from "@/features/agents/SessionBits";
import { useSessionWorktrees, worktreeSession } from "@/features/agents/Worktrees";
import { reviewWithAgent } from "@/features/github/reviewAgent";
import { useInbox } from "@/features/inbox/data";
import { MessageCard, useOutboxMessages } from "@/features/outbox/MessageCard";
import { ProposalCard } from "@/features/proposals/ProposalCard";
import { SuggestionCard } from "@/features/suggestions/Suggestions";
import { runAction } from "@/lib/actions";
import { formatAge } from "@/lib/format";
import { projectName, useProjects, useSession } from "@/lib/queries";
import { emptyLog, useSessionLogs } from "@/lib/sessionStore";
import { isDone } from "@/lib/sessionStatus";
import type { TodayItem } from "./data";

export const ICON: Record<TodayItem["kind"], IconName> = {
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

/** The primary branch of a session's worktree: its facts, and whether Merge is safe from a card (Done, its runner
 * here, branch on Home, nothing uncommitted, base unchanged). */
export function useBranch(sessionId: string, s: Session | null) {
  const worktrees = useSessionWorktrees(sessionId).data;
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
  return { w, t, base, safe, facts };
}

/** The branch line and Merge (only when safe) or Review changes. `onMerged` runs after a merge that went through. */
export function BranchActions({ sessionId, s, onMerged, children }: { sessionId: string; s: Session | null; onMerged?: () => Promise<unknown>; children?: ReactNode }) {
  const qc = useQueryClient();
  const { w, t, base, safe, facts } = useBranch(sessionId, s);
  // Merged: the card is handled. A merge that waits or stopped keeps the card, with Review changes.
  const merged = async (r: WorktreeInfo) => {
    void qc.invalidateQueries({ queryKey: ["worktrees"] });
    if (r.worktree.merged) await onMerged?.();
  };
  return <>
    {facts && <div className="flex min-w-0 items-center gap-1.5 text-[12px] text-ink-2"><Icon name="branch" size={13} className="flex-none text-ink-3" /><span className="truncate font-mono">{t!.branch}</span><span className="truncate">· {facts}</span></div>}
    <div className="flex flex-wrap items-center gap-1.5">
      {safe
        ? <ActionButton name="worktree.merge" params={{ id: t!.id }} vars={{ branch: t!.branch, base: base! }} query={{ session: worktreeSession(t!) ?? sessionId }} primary onDone={merged}>Merge into {base}</ActionButton>
        : w && (w.ahead > 0 || w.dirty_files > 0) && <Link className="btn btn-sm btn-primary no-underline" to="/agents" search={{ session: sessionId, view: "changes" }}>Review changes</Link>}
      {children}
    </div>
  </>;
}

/** A finished agent: its last message, its branch with Merge when safe, Archive and Mark seen. */
function FinishedCard({ item }: { item: TodayItem }) {
  const s = useSession(item.record);
  return <Card item={item} age={!s} meta={<SessionMeta s={s} item={item} />} open={<OpenSession id={item.record} />}>
    {s?.last_message && <p className="m-0 line-clamp-3 text-[13px] text-ink-2">{s.last_message}</p>}
    <BranchActions sessionId={item.record} s={s} onMerged={() => runAction("session.seen", { id: item.record }).catch(toastError)}>
      <ActionButton name="session.archive" params={{ id: item.record }} />
      <ActionButton name="session.seen" params={{ id: item.record }} title="Keep the agent and remove the card" />
    </BranchActions>
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

export const ItemCard = memo(function ItemCard({ item, proposal, suggestion }: { item: TodayItem; proposal: Proposal | undefined; suggestion: Suggestion | undefined }) {
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
