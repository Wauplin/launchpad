// Review (desktop sidebar entry and phone tab): everything that waits for an owner decision, in three groups.
// To publish (suggestions and Local PRs), Agent branches (worktrees to review or merge) and Review requests
// (GitHub PRs that request your review). Nothing is posted, published or merged from this list: each row opens
// the page with the explicit button. Discard only drops local drafts, after a confirmation.

import { useQueryClient, useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useMemo, useRef, type ReactNode } from "react";
import { create } from "zustand";
import type { Predraft } from "@/gen/Predraft";
import type { Project } from "@/gen/Project";
import type { Session } from "@/gen/Session";
import type { Suggestion } from "@/gen/Suggestion";
import type { Thread } from "@/gen/Thread";
import type { WorktreeInfo } from "@/gen/WorktreeInfo";
import { Icon, type IconName } from "@/components/Icon";
import { toast, toastError } from "@/components/Toast";
import { Badge, ConfirmButton, Empty, FilterChip, FilterChips, Loading, PageHeader } from "@/components/ui";
import { reviewWithAgent } from "@/features/github/reviewAgent";
import { api } from "@/lib/api";
import { formatAge } from "@/lib/format";
import { usePhone } from "@/lib/phone";
import { useProjects, useSessions } from "@/lib/queries";
import { itemSuggestionKey, reviewGroups, useReviewData, useSuggestionsByItem, type ReviewGroup } from "./data";

const useGroup = create<{ group: ReviewGroup }>(() => ({ group: "publish" }));
const eventName = (e: string) => (e === "APPROVE" ? "approve" : e === "REQUEST_CHANGES" ? "request changes" : "comment");
const ghParams = (repo: string, number: number) => ({ owner: repo.split("/")[0], repo: repo.split("/")[1], number: String(number) });

/** The one row design: icon tile, title, one meta line, then one primary action and at most one more. */
function ReviewRow({ icon, tone = "", title, meta, primary, secondary }: { icon: IconName; tone?: string; title: ReactNode; meta: ReactNode; primary?: ReactNode; secondary?: ReactNode }) {
  const phone = usePhone();
  return <div className={`review-row grid items-center gap-x-3 border-b border-line-2 px-4 ${phone ? "h-[112px] grid-cols-[32px_minmax(0,1fr)] content-center gap-y-2" : "h-14 grid-cols-[32px_minmax(0,1fr)_auto]"}`}>
    <span className={`grid size-8 place-items-center rounded-lg bg-sunk ${tone || "text-ink-2"}`}><Icon name={icon} size={15} /></span>
    <div className="min-w-0">
      <div className="truncate text-[13px] font-semibold">{title}</div>
      <div className="mt-0.5 flex min-w-0 items-center gap-1.5 truncate text-[12px] text-ink-3">{meta}</div>
    </div>
    {(primary || secondary) && <div className={`flex items-center justify-end gap-2 ${phone ? "col-span-2 justify-start" : ""}`}>{primary}{secondary}</div>}
  </div>;
}

const SuggestionRow = memo(function SuggestionRow({ s, session, predraft }: { s: Suggestion; session: Session | undefined; predraft: Predraft | undefined }) {
  const qc = useQueryClient();
  const t = s.target;
  const where = t.kind === "git_hub" ? `${t.repo}#${t.number}` : `Local PR "${predraft?.title ?? t.id}"`;
  const uncertain = s.status === "posting" || s.status === "uncertain";
  const discard = async () => {
    try { await api.delete(`/api/suggestions/${s.id}`); toast(`Suggestion on ${where} discarded`); }
    catch (e) { toastError(e); }
    finally { void qc.invalidateQueries({ queryKey: ["suggestions"] }); }
  };
  const open = t.kind === "git_hub"
    ? <Link className="btn btn-sm btn-primary no-underline" to="/github/$owner/$repo/$number" params={ghParams(t.repo, t.number)} search={{ draft: s.id }}>Review &amp; post</Link>
    : <Link className="btn btn-sm btn-primary no-underline" to="/predrafts/$id" params={{ id: t.id }}>Review &amp; post</Link>;
  return <ReviewRow
    icon="comment" tone={s.kind === "review" ? "text-attn" : "text-teal-ink"}
    title={s.kind === "review" ? `Review on ${where}` : `${t.kind === "predraft" ? "Note" : "Comment"} on ${where}`}
    meta={<>
      <span className="flex-none">Suggestion{s.kind === "review" ? ` · ${eventName(s.event)}` : ""}</span>
      {uncertain && <Badge tone="attn" icon="alert" title="Publication was attempted: check GitHub before discarding">check GitHub</Badge>}
      <span className="truncate">· {session ? `by "${session.title}"` : `by ${s.author}`}</span>
      {s.head && <span className="flex-none font-mono" title={s.head}>· {s.head.slice(0, 7)}</span>}
      <span className="flex-none">· {formatAge(s.created_at)} ago</span>
    </>}
    primary={open}
    secondary={<ConfirmButton disabled={s.status === "posting"} consequence={uncertain ? "Check GitHub first: it may already be posted." : "Deletes the local suggestion."} confirmLabel="Discard" onConfirm={discard}>Discard</ConfirmButton>}
  />;
});

const PredraftRow = memo(function PredraftRow({ p, project, session }: { p: Predraft; project: Project | undefined; session: Session | undefined }) {
  const qc = useQueryClient();
  const discard = async () => {
    try { await api.post(`/api/predrafts/${p.id}/discard`); toast(`Local PR "${p.title}" discarded`); }
    catch (e) { toastError(e); }
    finally { void qc.invalidateQueries({ queryKey: ["predrafts"] }); }
  };
  return <ReviewRow
    icon="pr-local" tone="text-teal-ink" title={p.title}
    meta={<>
      <span className="flex-none">Local PR · {project?.name ?? "folder removed"}</span>
      <span className="truncate font-mono">· {p.branch}</span>
      {session && <span className="truncate">· by "{session.title}"</span>}
      <span className="flex-none">· {formatAge(p.created_at)} ago</span>
    </>}
    primary={<Link className="btn btn-sm btn-primary no-underline" to="/predrafts/$id" params={{ id: p.id }}>Review &amp; publish</Link>}
    secondary={<ConfirmButton consequence="Deletes the branch and the worktree." confirmLabel="Discard" onConfirm={discard}>Discard</ConfirmButton>}
  />;
});

const BranchRow = memo(function BranchRow({ i, session }: { i: WorktreeInfo; session: Session | undefined }) {
  const w = i.worktree;
  const state = w.merge ? "merge requested" : w.state === "handed_off" ? "in the checkout" : null;
  return <ReviewRow
    icon="branch" title={session?.title ?? w.branch}
    meta={<>
      <span className="flex-none">{i.project_name}</span>
      <span className="truncate font-mono">· {w.branch}</span>
      {i.ahead > 0 && <span className="flex-none">· {i.ahead} commit{i.ahead === 1 ? "" : "s"} ahead</span>}
      {i.dirty_files > 0 && <span className="flex-none">· {i.dirty_files} uncommitted</span>}
      {state && <Badge tone="attn">{state}</Badge>}
    </>}
    primary={session && <Link className="btn btn-sm btn-primary no-underline" to="/agents" search={{ session: session.id, view: "changes" }}>Review changes</Link>}
    secondary={session && <Link className="btn btn-sm no-underline" to="/agents" search={{ session: session.id }}>Open agent</Link>}
  />;
});

const RequestRow = memo(function RequestRow({ t, ready, projects }: { t: Thread; ready: Suggestion | undefined; projects: Project[] }) {
  const n = t.number!;
  return <ReviewRow
    icon="pr" tone="text-gh-open" title={<span className={t.unread ? "" : "font-normal"}>{t.title}</span>}
    meta={<>
      <span className="truncate">{t.repo} #{n}</span>
      <span className="flex-none">· {formatAge(Date.parse(t.updated_at))} ago</span>
      {ready && <Badge tone="teal" icon="comment">Suggestion ready</Badge>}
    </>}
    primary={ready
      ? <Link className="btn btn-sm btn-primary no-underline" to="/github/$owner/$repo/$number" params={ghParams(t.repo, n)} search={{ draft: ready.id }}>Review &amp; post</Link>
      : <button className="btn btn-sm btn-primary" title="Opens a launch draft that asks an agent for a local suggested review" onClick={() => reviewWithAgent({ projects, repo: t.repo, number: n, title: t.title, url: t.url })}>Review with agent</button>}
    secondary={<Link className="btn btn-sm no-underline" to="/github/$owner/$repo/$number" params={ghParams(t.repo, n)}>Open PR</Link>}
  />;
});

const empty: Record<ReviewGroup, string> = {
  publish: "No suggestion or Local PR waits for you.",
  branches: "No agent branch waits for a review or a merge.",
  requests: "No pull request requests your review.",
};

export function ReviewView() {
  const phone = usePhone();
  const group = useGroup((s) => s.group);
  const data = useReviewData();
  const projects = useProjects().data ?? [];
  const sessions = useSessions().data;
  const predrafts = useQuery({ queryKey: ["predrafts"], queryFn: () => api.get<Predraft[]>("/api/predrafts") }).data;
  const byItem = useSuggestionsByItem();
  const bySession = useMemo(() => new Map((sessions ?? []).map((s) => [s.id, s])), [sessions]);
  const counts: Record<ReviewGroup, number> = { publish: data.publish.length, branches: data.branches.length, requests: data.requests.length };
  const count = counts[group];
  const ref = useRef<HTMLDivElement>(null);
  const key = (i: number) => group === "publish" ? (() => { const r = data.publish[i]; return r.kind === "suggestion" ? r.s.id : r.p.id; })() : group === "branches" ? data.branches[i].worktree.id : data.requests[i].id;
  const v = useVirtualizer({ count, getScrollElement: () => ref.current, estimateSize: () => (phone ? 112 : 56), getItemKey: key, overscan: 4, useFlushSync: false });
  const row = (i: number) => {
    if (group === "publish") {
      const r = data.publish[i];
      if (r.kind === "predraft") return <PredraftRow p={r.p} project={projects.find((x) => x.id === r.p.project_id)} session={r.p.session_id ? bySession.get(r.p.session_id) : undefined} />;
      const t = r.s.target;
      return <SuggestionRow s={r.s} session={r.s.session_id ? bySession.get(r.s.session_id) : undefined} predraft={t.kind === "predraft" ? predrafts?.find((p) => p.id === t.id) : undefined} />;
    }
    if (group === "branches") { const b = data.branches[i]; return <BranchRow i={b} session={bySession.get(b.worktree.sessions[0])} />; }
    const t = data.requests[i];
    return <RequestRow t={t} ready={byItem.get(itemSuggestionKey(t.repo, t.number))?.find((s) => s.kind === "review")} projects={projects} />;
  };
  const chips = <FilterChips tour="review.groups" label="Review groups">
    {reviewGroups.map(([id, label, title]) => <FilterChip key={id} active={group === id} count={counts[id]} title={title} onClick={() => { useGroup.setState({ group: id }); ref.current?.scrollTo({ top: 0 }); }}>{label}</FilterChip>)}
  </FilterChips>;
  const note = "Nothing is posted or published without your click";
  return <div className="flex h-full min-h-0 flex-col">
    {phone ? <div className="phone-page-content grid gap-1.5 border-b border-line-2 py-2"><span className="text-xs text-ink-3">{note}</span>{chips}</div>
      : <PageHeader title="Review" meta={<span>{note}</span>} tabs={<div className="pb-3">{chips}</div>} />}
    <div ref={ref} className="min-h-0 flex-1 overflow-y-auto">
      {count === 0 && (group === "branches" && !data.branchesLoaded ? <Loading what="agent branches" /> : <Empty fill icon="check">{empty[group]}</Empty>)}
      <div className="relative" style={{ height: v.getTotalSize() }}>
        {v.getVirtualItems().map((item) => <div key={item.key} className="absolute top-0 left-0 w-full" style={{ transform: `translateY(${item.start}px)` }}>{row(item.index)}</div>)}
      </div>
    </div>
  </div>;
}
