// Review (phone tab, also reachable on the desktop at /review): what waits for an owner decision. Agent
// suggestions and pre-draft PRs to publish, agent branches to review and merge, and GitHub PRs that request
// your review. Nothing is posted or merged from a list: each row opens the page with the explicit button.

import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import type { ReactNode } from "react";
import type { Predraft } from "@/gen/Predraft";
import type { WorktreeInfo } from "@/gen/WorktreeInfo";
import { Icon } from "@/components/Icon";
import { WorktreeCard } from "@/features/agents/Worktrees";
import { useInboxThreads } from "@/features/inbox/data";
import { usePublishItems } from "@/features/publish/items";
import { api } from "@/lib/api";
import { formatAge } from "@/lib/format";
import { useProjects, useSessions } from "@/lib/queries";
import { useScope } from "@/lib/workspace";

function Section({ title, count, empty, children }: { title: string; count: number; empty: string; children: ReactNode }) {
  return <section className="grid gap-2">
    <h2 className="seclabel px-1">{title} · {count}</h2>
    {count ? children : <p className="px-1 text-ink-3">{empty}</p>}
  </section>;
}

const card = "grid gap-2 rounded-xl border border-line bg-paper p-3";
const event = (e: string) => (e === "APPROVE" ? "Approve" : e === "REQUEST_CHANGES" ? "Request changes" : "Comment");

export function ReviewView() {
  const scope = useScope();
  const items = usePublishItems();
  const projects = useProjects().data ?? [];
  const sessions = useSessions().data ?? [];
  const predrafts = useQuery({ queryKey: ["predrafts"], queryFn: () => api.get<Predraft[]>("/api/predrafts") }).data ?? [];
  const worktrees = useQuery({ queryKey: ["worktrees", "all"], queryFn: () => api.get<WorktreeInfo[]>("/api/worktrees"), refetchInterval: 15_000 }).data;
  const { threads } = useInboxThreads();
  // Agent branches with work to look at: commits ahead of their base, uncommitted files, or a merge in progress.
  const branches = (worktrees ?? [])
    .filter((i) => scope.hasProject(i.worktree.project_id) && (i.worktree.state === "active" || i.worktree.state === "handed_off") && (i.ahead > 0 || i.dirty_files > 0 || !!i.worktree.merge))
    .sort((a, b) => b.worktree.created_at - a.worktree.created_at);
  const requests = threads.filter((t) => t.reason === "review_requested" && t.kind === "PullRequest" && t.number);
  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto grid max-w-[720px] gap-6 px-3 py-4">
        <Section title="To publish" count={items.length} empty="No agent drafts or pre-draft PRs wait for you.">
          {items.map((r) => {
            if (r.kind === "predraft") {
              const p = r.p;
              return <article key={p.id} className={card}>
                <div className="flex min-w-0 items-start gap-2"><Icon name="pr-local" size={16} className="mt-0.5 text-ink-3" /><div className="min-w-0 flex-1"><div className="font-semibold">{p.title}</div><div className="truncate text-[12.5px] text-ink-3">{projects.find((x) => x.id === p.project_id)?.name ?? "missing project"} · <span className="font-mono">{p.branch}</span> · {formatAge(p.created_at)} ago</div></div></div>
                <div className="flex flex-wrap gap-2"><Link className="btn btn-primary no-underline" to="/predrafts/$id" params={{ id: p.id }}>Review &amp; publish</Link>{p.session_id && <Link className="btn no-underline" to="/agents" search={{ session: p.session_id }}>Agent</Link>}</div>
              </article>;
            }
            const s = r.s;
            const t = s.target;
            const where = t.kind === "git_hub" ? `${t.repo} #${t.number}` : `pre-draft "${predrafts.find((p) => p.id === t.id)?.title ?? t.id}"`;
            return <article key={s.id} className={card}>
              <div className="flex min-w-0 items-start gap-2"><Icon name="comment" size={16} className="mt-0.5 text-ink-3" /><div className="min-w-0 flex-1"><div className="font-semibold">{s.kind === "review" ? `Review on ${where} · ${event(s.event)}` : `Comment on ${where}`}</div><div className="truncate text-[12.5px] text-ink-3">{s.head && <>inspected <span className="font-mono">{s.head.slice(0, 7)}</span> · </>}{formatAge(s.created_at)} ago{s.status !== "draft" ? ` · publication ${s.status}: check GitHub` : ""}</div></div></div>
              <p className="line-clamp-3 text-[13px] text-ink-2">{s.body}</p>
              <div className="flex flex-wrap gap-2">{t.kind === "git_hub"
                ? <Link className="btn btn-primary no-underline" to="/github/$owner/$repo/$number" params={{ owner: t.repo.split("/")[0], repo: t.repo.split("/")[1], number: String(t.number) }} search={{ draft: s.id }}>Review &amp; post</Link>
                : <Link className="btn btn-primary no-underline" to="/predrafts/$id" params={{ id: t.id }}>Review &amp; post</Link>}</div>
            </article>;
          })}
        </Section>
        <Section title="Agent branches" count={branches.length} empty={worktrees ? "No agent branch waits for a review or a merge." : "Loading…"}>
          {branches.map((i) => {
            const session = sessions.find((s) => s.id === i.worktree.sessions[0]);
            return <article key={i.worktree.id} className={card}>
              {session && <div className="truncate font-semibold" title={session.title}>{session.title}</div>}
              <WorktreeCard info={i} session={session} />
              {session && <div className="flex flex-wrap gap-2 border-t border-line-2 pt-2"><Link className="btn no-underline" to="/agents" search={{ session: session.id, view: "changes" }}>Review changes</Link><Link className="btn no-underline" to="/agents" search={{ session: session.id }}>Agent</Link></div>}
            </article>;
          })}
        </Section>
        <Section title="Review requests" count={requests.length} empty="No pull request requests your review.">
          {requests.map((t) => {
            const [owner, repo] = t.repo.split("/");
            return <Link key={t.id} to="/github/$owner/$repo/$number" params={{ owner, repo, number: String(t.number) }} className={`${card} text-ink no-underline`}>
              <div className="flex min-w-0 items-start gap-2"><Icon name="pr" size={16} className="mt-0.5 text-gh-open" /><div className="min-w-0 flex-1"><div className={t.unread ? "font-semibold" : ""}>{t.title}</div><div className="truncate text-[12.5px] text-ink-3">{t.repo} #{t.number} · {formatAge(Date.parse(t.updated_at))} ago</div></div></div>
            </Link>;
          })}
        </Section>
      </div>
    </div>
  );
}
