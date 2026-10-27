// The groups of the Review page (To publish · Agent branches · Review requests), kept apart from the page so the
// sidebar entry and the phone tab badge count exactly what the page lists without loading it.

import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import type { Suggestion } from "@/gen/Suggestion";
import type { Thread } from "@/gen/Thread";
import type { WorktreeInfo } from "@/gen/WorktreeInfo";
import { useInboxThreads } from "@/features/inbox/data";
import { usePublishItems, type Row } from "@/features/publish/items";
import { api } from "@/lib/api";
import { useScope } from "@/lib/workspace";

export type ReviewGroup = "publish" | "branches" | "requests";
export const reviewGroups: [ReviewGroup, string, string][] = [
  ["publish", "To publish", "Suggestions and Local PRs that wait for your click"],
  ["branches", "Agent branches", "Agent worktrees with commits, uncommitted files or a merge request"],
  ["requests", "Review requests", "Open pull requests where GitHub requests your review"],
];

export const useSuggestionList = () => useQuery({ queryKey: ["suggestions"], queryFn: () => api.get<Suggestion[]>("/api/suggestions") });

/** Unposted suggestions of a GitHub item, keyed `owner/repo#n` (lowercase repo). Reviews come first. */
export function useSuggestionsByItem() {
  const all = useSuggestionList().data;
  return useMemo(() => {
    const out = new Map<string, Suggestion[]>();
    for (const s of all ?? []) {
      if (s.status === "posted" || s.target.kind !== "git_hub") continue;
      const k = `${s.target.repo.toLowerCase()}#${s.target.number}`;
      out.set(k, [...(out.get(k) ?? []), s].sort((a, b) => (a.kind === "review" ? 0 : 1) - (b.kind === "review" ? 0 : 1)));
    }
    return out;
  }, [all]);
}
export const itemSuggestionKey = (repo: string, number: number | null) => `${repo.toLowerCase()}#${number}`;

export type ReviewData = { publish: Row[]; branches: WorktreeInfo[]; requests: Thread[]; branchesLoaded: boolean; total: number };

/** Everything the Review page lists, scoped to the window's workspace. `total` is the sidebar and phone badge count. */
export function useReviewData(): ReviewData {
  const scope = useScope();
  const publish = usePublishItems();
  const worktrees = useQuery({ queryKey: ["worktrees", "all"], queryFn: () => api.get<WorktreeInfo[]>("/api/worktrees"), refetchInterval: 15_000 }).data;
  const { threads } = useInboxThreads();
  // Agent branches with work to look at: commits ahead of their base, uncommitted files, or a merge in progress.
  const branches = useMemo(() => (worktrees ?? [])
    .filter((i) => scope.hasProject(i.worktree.project_id) && !i.worktree.predraft_id && (i.worktree.state === "active" || i.worktree.state === "handed_off") && (i.ahead > 0 || i.dirty_files > 0 || !!i.worktree.merge))
    .sort((a, b) => b.worktree.created_at - a.worktree.created_at), [worktrees, scope]);
  const requests = useMemo(() => threads.filter((t) => t.reason === "review_requested" && t.kind === "PullRequest" && t.number !== null && t.state !== "closed" && t.state !== "merged"), [threads]);
  return { publish, branches, requests, branchesLoaded: !!worktrees, total: publish.length + branches.length + requests.length };
}

/** The sidebar entry and the phone Review tab badge: the same groups as the page. */
export const useReviewCount = () => useReviewData().total;
