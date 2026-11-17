// Pure helpers about a branch's Local PR (no React), shared by the session view and tested in tests/localPr.test.mjs.

import type { Predraft } from "../../gen/Predraft";
import type { WorktreeInfo } from "../../gen/WorktreeInfo";

/** The GitHub pull request a Local PR was published as, from its URL. */
export function githubPr(p: Pick<Predraft, "published_url" | "published_number">): { repo: string; number: number } | null {
  const m = p.published_url?.match(/^https:\/\/github\.com\/([^/]+\/[^/]+)\/pull\/(\d+)/);
  if (!m) return null;
  return { repo: m[1], number: p.published_number ?? Number(m[2]) };
}

/** The Local PR of a worktree's branch: the one it records, else a Local PR of the same folder and branch that was
 * not discarded. */
export function localPrOf(w: Pick<WorktreeInfo["worktree"], "predraft_id" | "project_id" | "branch">, localPrs: Predraft[] | undefined): Predraft | undefined {
  if (!localPrs) return undefined;
  return (w.predraft_id ? localPrs.find((p) => p.id === w.predraft_id && !p.archived) : undefined)
    ?? localPrs.find((p) => !p.archived && p.project_id === w.project_id && p.branch === w.branch);
}

export type LocalPrStatus = "discarded" | "merged" | "published" | "open";

/** Where a Local PR stands. Merged wins over published: a pull request merged on GitHub stays linked but is done. */
export function localPrStatus(p: Pick<Predraft, "archived" | "merged" | "published_url">): LocalPrStatus {
  if (p.archived) return "discarded";
  if (p.merged) return "merged";
  if (p.published_url) return "published";
  return "open";
}

/** One line on a published Local PR: Home pushes new commits only to a pull request it opened, never to a linked one
 * or a merged one. */
export function publishedText(p: Pick<Predraft, "linked_at" | "merged">): string {
  if (p.merged) return "New commits on this branch need a new Local PR.";
  if (p.linked_at) return "Opened outside Launchpad and linked here: Home does not push this branch to it.";
  return "New commits of agents on this branch reach the pull request through Home.";
}
