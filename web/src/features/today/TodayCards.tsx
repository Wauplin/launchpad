// The cards of Today, one per kind of decision, each with its main action from the action list. The bot chat
// shows the same cards for what a bot's session wrote (ready clicks) and the same branch actions (Changes).

import { useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { memo, useEffect, useMemo, type ReactNode } from "react";
import type { Proposal } from "@/gen/Proposal";
import type { Session } from "@/gen/Session";
import type { Suggestion } from "@/gen/Suggestion";
import type { WorktreeInfo } from "@/gen/WorktreeInfo";
import { ActionButton, ActionMenuItem } from "@/components/ActionButton";
import { HarnessMark } from "@/components/HarnessMark";
import { Icon, type IconName } from "@/components/Icon";
import { toastError } from "@/components/Toast";
import { Loading, MoreMenu } from "@/components/ui";
import { AskCard } from "@/features/agents/AskCard";
import { buildRows } from "@/features/agents/rows";
import { SessionStatus } from "@/features/agents/SessionBits";
import { useSessionWorktrees, worktreeSession } from "@/features/agents/Worktrees";
import { reviewWithAgent } from "@/features/github/reviewAgent";
import { MessageCard, useOutboxMessages } from "@/features/outbox/MessageCard";
import { ProposalCard } from "@/features/proposals/ProposalCard";
import { SuggestionCard } from "@/features/suggestions/Suggestions";
import { runAction } from "@/lib/actions";
import { ago } from "@/lib/format";
import { isPhone } from "@/lib/phone";
import { projectName, useProjects, useSession } from "@/lib/queries";
import { emptyLog, useSessionLogs } from "@/lib/sessionStore";
import { isDone } from "@/lib/sessionStatus";
import type { TodayItem } from "./data";

export const ICON: Record<TodayItem["kind"], IconName> = {
  question: "alert", finished: "check", failed: "xcircle", local_pr: "pr-local", proposal: "spark", outbox: "send", suggestion: "comment", review_request: "pr", routine: "refresh", runner: "window",
};

export const ghParams = (repo: string, number: number) => ({ owner: repo.split("/")[0], repo: repo.split("/")[1], number: String(number) });

/** The card frame: icon, title, one meta line, then the kind's own content and buttons. `age: false` when the meta already says since when (a session's status badge). */
function Card({ item, meta, open, children, tone = "border-line", age = true }: { item: TodayItem; meta?: ReactNode; open?: ReactNode; children?: ReactNode; tone?: string; age?: boolean }) {
  return (
    <article data-tour="today.card" data-tour-id={item.id} className={`group grid min-w-0 grid-cols-1 gap-2.5 rounded-card border bg-paper p-3 ${tone}`}>
      <div className="flex min-w-0 items-start gap-2.5">
        <span className="grid size-7 flex-none place-items-center rounded-card bg-sunk text-ink-2"><Icon name={ICON[item.kind]} size={14} /></span>
        <div className="min-w-0 flex-1">
          <div className="line-clamp-2 text-[13px] font-semibold" title={item.title}>{item.title}</div>
          <div className="flex min-w-0 flex-wrap items-center gap-x-1.5 text-[12px] text-ink-3">{meta ?? <span className="truncate">{item.detail}</span>}{age && <span className="flex-none">· {ago(item.at)}</span>}</div>
        </div>
        {open && <span className="flex-none">{open}</span>}
      </div>
      {children}
    </article>
  );
}

export const OpenSession = ({ id, view }: { id: string; view?: "changes" }) => <Link className="btn btn-sm no-underline" to="/agents" search={{ session: id, ...(view ? { view } : {}) }}>Open</Link>;

export function SessionMeta({ s, item }: { s: Session | null; item: TodayItem }) {
  const projects = useProjects().data;
  if (!s) return <span className="truncate">{item.detail}</span>;
  // The phone's line keeps the room for the folder and status; the harness shows on the agent's page.
  return <>{!isPhone && <HarnessMark harness={s.harness} size={13} />}<span className="truncate">{projectName(projects, s.project_id, s.cwd)}</span><SessionStatus s={s} /></>;
}

/** Loads the newest events of a session (the same tail page the session view uses). */
export function useTail(id: string) {
  const log = useSessionLogs((st) => st.logs[id]);
  const load = useSessionLogs((st) => st.load);
  useEffect(() => {
    if (log) return;
    useSessionLogs.setState((st) => ({ logs: { ...st.logs, [id]: emptyLog() } }));
    void load(id);
  }, [id, log, load]);
  return log;
}

/** The open question of a session, from its newest events. */
export function useOpenQuestion(id: string) {
  const log = useTail(id);
  return useMemo(() => {
    const rows = log ? buildRows(log) : [];
    for (let i = rows.length - 1; i >= 0; i--) { const r = rows[i]; if (r.t === "ask" && !r.answers) return r; }
    return null;
  }, [log]);
}

/** Answer in place: the open question's options, as in the session. */
function QuestionCard({ item }: { item: TodayItem }) {
  const s = useSession(item.record);
  const ask = useOpenQuestion(item.record);
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

/** The session page's main action is Merge (laptop header, phone bottom bar): safe, finished and no open question.
 * Next steps then leave Merge out of that worktree's card. */
export const mergeIsMain = (s: Session, branch: ReturnType<typeof useBranch>) => branch.safe && s.status === "waiting" && !s.open_question;

/** Merge the branch into its base (call only when `useBranch(…).safe`). `onMerged` runs after a merge that went
 * through; a merge that waits or stopped keeps the card, with Review changes. */
export function MergeButton({ sessionId, branch: { t, base }, onMerged, className, label, primary = true }: { sessionId: string; branch: ReturnType<typeof useBranch>; onMerged?: () => Promise<unknown>; className?: string; label?: string; primary?: boolean }) {
  const qc = useQueryClient();
  const merged = async (r: WorktreeInfo) => {
    void qc.invalidateQueries({ queryKey: ["worktrees"] });
    if (r.worktree.merged) await onMerged?.();
  };
  return <ActionButton name="worktree.merge" params={{ id: t!.id }} vars={{ branch: t!.branch, base: base! }} query={{ session: worktreeSession(t!) ?? sessionId }} primary={primary} title={`Merge ${t!.branch} into ${base}`} className={className} onDone={merged}>{label ?? `Merge into ${base}`}</ActionButton>;
}

/** The branch line and Merge (only when safe) or Review changes. `onMerged` runs after a merge that went through. */
export function BranchActions({ sessionId, s, onMerged, children }: { sessionId: string; s: Session | null; onMerged?: () => Promise<unknown>; children?: ReactNode }) {
  const branch = useBranch(sessionId, s);
  const { w, t, safe, facts } = branch;
  return <>
    {facts && <div className="flex min-w-0 items-center gap-1.5 text-[12px] text-ink-2"><Icon name="branch" size={12} className="flex-none text-ink-3" /><span className="truncate font-mono">{t!.branch}</span><span className="truncate">· {facts}</span></div>}
    <div className="flex flex-wrap items-center gap-1.5">
      {safe
        ? <MergeButton sessionId={sessionId} branch={branch} onMerged={onMerged} />
        : w && (w.ahead > 0 || w.dirty_files > 0) && <Link className="btn btn-sm btn-primary no-underline" to="/agents" search={{ session: sessionId, view: "changes" }}>Review changes</Link>}
      {children}
    </div>
  </>;
}

/** Archive and Mark seen in ⋯, next to the card's one main action. `archiveMain`: nothing to merge or review, so
 * Archive is the main action and stays visible. */
export function Later({ id, archiveMain = false }: { id: string; archiveMain?: boolean }) {
  return <>
    {archiveMain && <ActionButton name="session.archive" params={{ id }} primary />}
    <MoreMenu label="More actions">{(close) => <>
      {!archiveMain && <ActionMenuItem name="session.archive" params={{ id }} close={close} detail="It stays resumable" />}
      <ActionMenuItem name="session.seen" params={{ id }} close={close} detail="Keep the agent and remove the card" />
    </>}</MoreMenu>
  </>;
}

/** A finished agent: its last message, its branch with Merge when safe, Archive and Mark seen. */
function FinishedCard({ item }: { item: TodayItem }) {
  const s = useSession(item.record);
  // Merge or Review changes when there is work (BranchActions), else Archive.
  const loading = useSessionWorktrees(item.record).isPending;
  const { w, safe } = useBranch(item.record, s);
  const archiveMain = !loading && !safe && !(w && (w.ahead > 0 || w.dirty_files > 0));
  return <Card item={item} age={!s} meta={<SessionMeta s={s} item={item} />} open={<OpenSession id={item.record} />}>
    {s?.last_message && <p className="m-0 line-clamp-3 text-[13px] text-ink-2">{s.last_message}</p>}
    <BranchActions sessionId={item.record} s={s} onMerged={() => runAction("session.seen", { id: item.record }).catch(toastError)}>
      <Later id={item.record} archiveMain={archiveMain} />
    </BranchActions>
  </Card>;
}

function FailedCard({ item }: { item: TodayItem }) {
  const s = useSession(item.record);
  return <Card item={item} tone="border-gh-closed/40" age={!s} meta={<SessionMeta s={s} item={item} />} open={<OpenSession id={item.record} />}>
    {s?.last_message && <p className="m-0 line-clamp-3 text-[13px] text-ink-2">{s.last_message}</p>}
    <div className="flex flex-wrap items-center gap-1.5">
      <ActionButton name="session.resume" params={{ id: item.record }} body={{ continue_turn: true }} primary />
      <Later id={item.record} />
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
      <span>· {ago(s.created_at)}</span>
      {s.session_id && <Link to="/agents" search={{ session: s.session_id }} className="text-teal-ink no-underline hover:underline">Open agent</Link>}
    </div>
    <SuggestionCard suggestion={s} />
  </div>;
}

/** `owner/repo#number` of an item, from `TodayItem.github`. */
export function githubRef(item: TodayItem) {
  const m = item.github?.match(/^([^/]+\/[^#]+)#(\d+)$/);
  return m ? { repo: m[1], number: Number(m[2]) } : null;
}

/** Review with agent: a launch draft that asks an agent for a local suggested review of the pull request. */
export function ReviewWithAgent({ item, primary = true, label = "Review with agent" }: { item: TodayItem; primary?: boolean; label?: string }) {
  const projects = useProjects().data ?? [];
  const ref = githubRef(item);
  if (!ref) return null;
  return <button className={`btn btn-sm ${primary ? "btn-primary" : ""}`} title="Opens a launch draft: an agent reviews the pull request and writes a suggested review, which comes back to Today for you to edit and post" onClick={() => reviewWithAgent({ projects, ...ref, title: item.title, url: `https://github.com/${ref.repo}/pull/${ref.number}` })}><Icon name="spark" size={12} />{label}</button>;
}

function ReviewRequestCard({ item }: { item: TodayItem }) {
  const ref = githubRef(item);
  return <Card item={item} open={ref && <Link className="btn btn-sm no-underline" to="/github/$owner/$repo/$number" params={ghParams(ref.repo, ref.number)}>Open</Link>}>
    <div><ReviewWithAgent item={item} /></div>
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
