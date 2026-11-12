import type { Predraft } from "../../gen/Predraft";
import type { WorktreeInfo } from "../../gen/WorktreeInfo";

/** An unpublished Local PR still allows merging its worktree into the base on Home. */
export function canMergeWorktree(i: WorktreeInfo, localPrs?: Predraft[]): boolean {
  const w = i.worktree;
  const localPr = w.predraft_id ? localPrs?.find((p) => p.id === w.predraft_id) : undefined;
  // Wait for the Local PR record so a published branch never offers a local merge.
  if (w.predraft_id && (!localPr || localPr.published_url)) return false;
  const onHome = w.state === "active" || (w.state === "removed" && i.on_home);
  return onHome && !w.merge && !!w.base_branch && i.ahead > 0 && !w.pull_request && !!i.head;
}
