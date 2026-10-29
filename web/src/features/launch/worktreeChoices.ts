import type { Session } from "@/gen/Session";
import type { WorktreeInfo } from "@/gen/WorktreeInfo";
import { isRunning } from "../../lib/sessionStatus.ts";

/** Reuse the existing session-folder launch path so follow-ups retain managed worktree ownership. */
export function reusableWorktrees(worktrees: WorktreeInfo[], sessions: Session[], projectId: string | null) {
  return worktrees.flatMap(({ worktree: w, path, exists }) => {
    if (w.project_id !== projectId || !w.primary || w.state !== "active" || !exists || w.merged) return [];
    const source = sessions.find((s) => w.sessions.includes(s.id) && s.cwd === path && s.launch_spec.target.kind !== "browser");
    if (!source) return [];
    const busy = sessions.some((s) => w.sessions.includes(s.id) && (isRunning(s) || s.status === "in_terminal" || s.status === "quota_paused"));
    return [{ id: w.id, branch: w.branch, path, source, busy }];
  });
}
