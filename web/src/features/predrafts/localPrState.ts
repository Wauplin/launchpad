// Pure helpers about a branch's Local PR (no React), shared by the session view and tested in tests/localPr.test.mjs.

import type { Predraft } from "../../gen/Predraft";
import type { WorktreeInfo } from "../../gen/WorktreeInfo";

export type PrState = "draft" | "open" | "merged" | "closed";

/** The GitHub pull request a Local PR was published as, from its URL. */
export function githubPr(p: Pick<Predraft, "published_url" | "published_number">): { repo: string; number: number } | null {
  const m = p.published_url?.match(/^https:\/\/github\.com\/([^/]+\/[^/]+)\/pull\/(\d+)/);
  if (!m) return null;
  return { repo: m[1], number: p.published_number ?? Number(m[2]) };
}

/** A cached GitHub pull request's state in one word. */
export function prState(item: { state?: string; isDraft?: boolean; merged?: boolean }): PrState {
  if (item.merged || item.state === "MERGED") return "merged";
  if (item.state === "CLOSED") return "closed";
  return item.isDraft ? "draft" : "open";
}

/** The Local PR of a worktree's branch: the one it records, else a Local PR of the same folder and branch that was
 * not discarded. */
export function localPrOf(w: Pick<WorktreeInfo["worktree"], "predraft_id" | "project_id" | "branch">, localPrs: Predraft[] | undefined): Predraft | undefined {
  if (!localPrs) return undefined;
  return (w.predraft_id ? localPrs.find((p) => p.id === w.predraft_id && !p.archived) : undefined)
    ?? localPrs.find((p) => !p.archived && p.project_id === w.project_id && p.branch === w.branch);
}
