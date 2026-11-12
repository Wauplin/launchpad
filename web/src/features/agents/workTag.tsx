// Where an agent's work went, as one tag in its Agents row: merged, already in its base branch, in a Local PR or a
// pull request, or read-only. It answers "can I archive this?" without opening the session.

import type { Predraft } from "@/gen/Predraft";
import type { Session } from "@/gen/Session";
import type { WorktreeInfo } from "@/gen/WorktreeInfo";
import { isMergedWorktree } from "./Worktrees";
import { githubPr, localPrOf } from "@/features/predrafts/localPrState";

/** `localPr`: the Local PR the tag comes from; `pr`: the GitHub pull request it was published or pushed to. The row's
 * ⋯ opens them (the row itself is one button that opens the session). */
export type WorkTag = { kind: "merged" | "in_base" | "local_pr" | "pr" | "read_only"; label: string; title: string; localPr?: Predraft; pr?: { repo: string; number: number } };

/** The tag of each session, by id; sessions without one are left out. */
export function workTags(sessions: Session[], worktrees: WorktreeInfo[] | undefined, localPrs: Predraft[] | undefined): Map<string, WorkTag> {
  const bySession = new Map<string, WorktreeInfo[]>();
  for (const i of worktrees ?? []) for (const id of i.worktree.sessions) bySession.set(id, [...(bySession.get(id) ?? []), i]);
  const out = new Map<string, WorkTag>();
  for (const s of sessions) {
    const list = bySession.get(s.id) ?? [];
    const tag = workTag(s, list.find((i) => i.worktree.primary) ?? list[0], localPrs);
    if (tag) out.set(s.id, tag);
  }
  return out;
}

function workTag(s: Session, i: WorktreeInfo | undefined, localPrs: Predraft[] | undefined): WorkTag | null {
  if (s.launch_spec.worktree?.when_done === "read_only") return { kind: "read_only", label: "Read-only", title: "Launched read-only: this agent changes no files" };
  const w = i?.worktree;
  const base = w?.base_branch ?? "the base branch";
  if (i && w && w.state !== "discarded") {
    const pr = localPrOf(w, localPrs);
    if (isMergedWorktree(i, localPrs)) return { kind: "merged", label: "Merged", title: `Merged into ${w.merged?.into ?? base}`, localPr: pr, pr: pr ? githubPr(pr) ?? undefined : undefined };
    // A merged Local PR with newer commits on the branch holds unmerged work: no tag.
    const fromPr = pr?.merged ? null : prTag(pr);
    if (fromPr) return fromPr;
    if (w.pull_request) return { kind: "pr", label: `PR #${w.pull_request.number}`, title: `Pushed to ${w.pull_request.repo}#${w.pull_request.number}`, pr: { repo: w.pull_request.repo, number: w.pull_request.number } };
    // Commits made, none missing from the base branch and nothing uncommitted: the work is in the base already.
    if (i.ahead === 0 && i.dirty_files === 0 && !!i.head && !!w.base_commit && i.head !== w.base_commit && w.base_branch) return { kind: "in_base", label: `On ${w.base_branch}`, title: `Every commit of ${w.branch} is in ${w.base_branch}` };
  }
  // A Local PR made from the session without a worktree record (its checkout, or a worktree Home no longer lists).
  if (i) return null;
  return prTag(localPrs?.find((p) => p.session_id === s.id && !p.archived));
}

function prTag(p: Predraft | undefined): WorkTag | null {
  if (!p) return null;
  const pr = githubPr(p) ?? undefined;
  if (p.merged) return { kind: "merged", label: "Merged", title: `Local PR "${p.title}" merged`, localPr: p, pr };
  if (p.published_url) return { kind: "pr", label: p.published_number ? `PR #${p.published_number}` : "PR", title: `Published to GitHub: ${p.published_url}`, localPr: p, pr };
  return { kind: "local_pr", label: "Local PR", title: `In Local PR "${p.title}", not published yet`, localPr: p };
}

const TONE: Record<WorkTag["kind"], string> = {
  merged: "bg-sunk text-gh-merged",
  in_base: "bg-sunk text-gh-merged",
  local_pr: "bg-teal-soft text-teal-ink",
  pr: "bg-teal-soft text-teal-ink",
  read_only: "bg-sunk text-ink-3",
};

/** The tag as a small pill at the start of the row's second line. */
export function WorkTagPill({ tag, className = "" }: { tag: WorkTag; className?: string }) {
  return <span title={tag.title} className={`inline-flex h-4 shrink-0 items-center rounded px-1.5 text-[11px] font-semibold whitespace-nowrap ${TONE[tag.kind]} ${className}`}>{tag.label}</span>;
}
