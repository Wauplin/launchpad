// Where a branch's Local PR stands, and the one-click ways to it: the session header chip, the worktree card's
// buttons and the Summary's next step share these. A published Local PR leads to its GitHub pull request first.

import { useNavigate } from "@tanstack/react-router";
import type { Predraft } from "@/gen/Predraft";
import { Icon, type IconName } from "@/components/Icon";
import { openGitHubItem } from "@/lib/githubLinks";
import { githubPr } from "./localPrState";

export { githubPr, localPrOf } from "./localPrState";

/** The label, icon and color of a Local PR: its GitHub pull request once published, else its local state. */
export function localPrLook(p: Predraft): { label: string; icon: IconName; color: string; title: string } {
  const pr = githubPr(p);
  if (pr) return { label: `PR #${pr.number} · published`, icon: "pr", color: "text-gh-open", title: `Published as ${pr.repo}#${pr.number}: open the pull request on GitHub` };
  if (p.merged) return { label: "Local PR · merged", icon: "pr-merged", color: "text-gh-merged", title: `Merged into ${p.base_branch} on Home: open the Local PR` };
  if (p.archived) return { label: "Local PR · discarded", icon: "pr-local", color: "text-ink-3", title: "Discarded: open the Local PR" };
  return { label: "Local PR · open", icon: "pr-local", color: "text-teal-ink", title: `Open the Local PR "${p.title}", not published to GitHub yet` };
}

/** Navigation to a Local PR and to its GitHub pull request (on GitHub). */
export function useLocalPrNav() {
  const navigate = useNavigate();
  return {
    local: (p: Predraft) => void navigate({ to: "/predrafts/$id", params: { id: p.id } }),
    github: (p: Predraft) => {
      const pr = githubPr(p);
      if (!pr) return;
      openGitHubItem(pr.repo, pr.number, { kind: "pr" });
    },
  };
}

/** The header chip, styled like the branch chip: one click opens the GitHub pull request once published, else the
 * Local PR. */
export function LocalPrChip({ pr, className = "" }: { pr: Predraft; className?: string }) {
  const look = localPrLook(pr);
  const nav = useLocalPrNav();
  return <button className={`inline-flex shrink-0 items-center gap-1 rounded-control border border-line bg-paper px-1.5 text-[11px] text-ink-2 hover:text-ink ${className}`} title={look.title} onClick={() => (githubPr(pr) ? nav.github(pr) : nav.local(pr))}>
    <Icon name={look.icon} size={12} className={look.color} />{look.label}
  </button>;
}

/** One line on the Local PR for cards and next steps. */
export function localPrText(p: Predraft): string {
  const pr = githubPr(p);
  if (pr) return `Published as ${pr.repo}#${pr.number}. New commits on this branch reach it through Home.`;
  if (p.merged) return `Its Local PR was merged into ${p.base_branch}.`;
  return `In the Local PR “${p.title}”, not published to GitHub yet.`;
}

/** Open PR #n (once published) and Open Local PR, as outline buttons. */
export function LocalPrButtons({ pr }: { pr: Predraft }) {
  const nav = useLocalPrNav();
  const published = githubPr(pr);
  const look = localPrLook(pr);
  return <>
    {published && <button className="btn btn-sm" title={`Open ${published.repo}#${published.number}`} onClick={() => nav.github(pr)}><Icon name={look.icon} size={12} className={look.color} />Open PR #{published.number}</button>}
    <button className="btn btn-sm" title={`Open the Local PR “${pr.title}”`} onClick={() => nav.local(pr)}><Icon name="pr-local" size={12} />Open Local PR</button>
  </>;
}
