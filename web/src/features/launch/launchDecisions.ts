// Pure decisions shared by launch submission and its tests. The composer store owns the draft.
import type { LaunchSpec } from "../../gen/LaunchSpec";
import type { Project } from "../../gen/Project";
import { pullRequestOf } from "./pullRequest.ts";

/** Makes `p` the target of a draft, and drops it from the context folders. */
export function withFolder(s: LaunchSpec, p: Project): LaunchSpec {
  // Git folders of Home always run in a fresh worktree.
  const worktree = p.home_git ? (s.worktree ?? { base: null, when_done: p.when_done }) : s.worktree;
  return { ...s, worktree, target: { kind: "project", project_id: p.id }, context_projects: s.context_projects.filter((c) => c.project_id !== p.id) };
}

/** Build the payload at submission time from the current draft. */
export function prepareLaunch(draft: LaunchSpec, workspace: string | null, gitTarget: boolean, target: Project | null | undefined, prOptOut: string | null): LaunchSpec {
  const pr = draft.target.kind === "project" && !!draft.worktree && gitTarget && target ? pullRequestOf(draft.text, draft.github_refs, [target.gh_repo, target.gh_upstream]) : null;
  const pullRequest = pr && prOptOut !== `${pr.repo}#${pr.number}` ? pr : undefined;
  return { ...draft, workspace_id: workspace || null, worktree: draft.target.kind === "project" && !gitTarget ? null : draft.worktree && { ...draft.worktree, pull_request: pullRequest } };
}

export type LaunchValidation = {
  editGone: boolean;
  editing: boolean;
  timing: "now" | "later" | "repeat";
  scheduled: boolean;
  busyWorktree: boolean;
  hasProfiles: boolean;
  spec: Pick<LaunchSpec, "harness" | "profile" | "target">;
  missingProfile: boolean;
  modelMissing: boolean;
  profileName: string;
  workspace: string | null;
  hasPrompt: boolean;
  budgetValid: boolean;
  timingValid: boolean;
};

/** First actionable problem, in the same order as the composer's checks. */
export function launchBlocker(v: LaunchValidation): string | null {
  if (v.editGone) return "This scheduled agent already started or was cancelled";
  if (v.editing && v.timing === "later" && !v.scheduled) return "Add a start condition, or choose Start now";
  if (v.busyWorktree) return "Stop the active agent before using this worktree";
  if (!v.hasProfiles) return "Add a profile in Settings → Profiles";
  if (!v.spec.harness || !v.spec.profile) return "Pick a profile in the options";
  if (v.missingProfile) return "The profile no longer exists";
  if (v.modelMissing) return `Choose a model for ${v.profileName}`;
  if (v.spec.target.kind === "none") return "Pick a folder";
  if (!v.workspace) return "Waiting for the workspace to load";
  if (!v.hasPrompt) return "Write a prompt";
  if (!v.budgetValid) return "Fix the cost limit";
  if (!v.timingValid) return "Fix the start or repeat options";
  return null;
}
