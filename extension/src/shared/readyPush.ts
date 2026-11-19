// Push to this PR targets of a pull request page, from Home's structured data only: agent branches with new commits
// (`ExtBranch.push_ready`) and the waiting ready cards an agent prepared (`ExtContext.ready_pushes`, `launchpad propose
// ready` with action `branch.push-to-pr`). Never read from a note's text. Pure: no DOM, so tests import it.

import type { ExtContext } from "@/gen/ExtContext";
import type { ExtReadyPush } from "@/gen/ExtReadyPush";

/** What one Push to this PR click sends: the branch at the commit shown, and the ready card it starts (if any). */
export type PushTarget = {
  branch: string;
  head: string;
  /** Commits since its agent started (push_ready only). */
  ahead: number | null;
  /** The ready card this push runs (started after the push). */
  card: string | null;
  session_id: string | null;
  title: string | null;
};

/** The key of one push (branch at commit): one click at a time per key, whatever widget shows it. */
export const pushKey = (t: { branch: string; head: string }) => `${t.branch}@${t.head.toLowerCase()}`;

/** The waiting ready card naming this branch at this commit (ready to push), if any. */
export function cardFor(ctx: Pick<ExtContext, "ready_pushes">, branch: string, head: string): ExtReadyPush | null {
  return ctx.ready_pushes.find((r) => r.state === "ready" && r.branch === branch && r.head.toLowerCase() === head.toLowerCase()) ?? null;
}

/** A ready card as a push target. */
export function fromCard(r: ExtReadyPush): PushTarget {
  return { branch: r.branch, head: r.head, ahead: null, card: r.id, session_id: r.session_id, title: r.title || null };
}

/** Every push the page offers, once per branch and commit: agent branches (with the card that names them), then the
 * ready cards whose branch Home has at their commit and that no agent branch covers. */
export function pushTargets(ctx: Pick<ExtContext, "branches" | "ready_pushes">): PushTarget[] {
  const out: PushTarget[] = [];
  const seen = new Set<string>();
  for (const b of ctx.branches) {
    const p = b.push_ready;
    if (!p || seen.has(pushKey(p))) continue;
    seen.add(pushKey(p));
    out.push({ branch: p.branch, head: p.head, ahead: p.ahead, card: cardFor(ctx, p.branch, p.head)?.id ?? null, session_id: p.session_id, title: p.title });
  }
  for (const r of ctx.ready_pushes) {
    if (r.state !== "ready" || seen.has(pushKey(r))) continue;
    seen.add(pushKey(r));
    out.push(fromCard(r));
  }
  return out;
}

/** Where the timeline shows each ready card: the newest note of the agent that prepared it (note ID → cards, newest
 * card first). Cards without a session or without a note of it are left to the sidebar box. */
export function cardsByNote(ctx: Pick<ExtContext, "notes" | "ready_pushes">): Map<string, ExtReadyPush[]> {
  const out = new Map<string, ExtReadyPush[]>();
  for (const r of ctx.ready_pushes) {
    if (!r.session_id) continue;
    const notes = ctx.notes.filter((n) => !n.by_owner && n.session_id === r.session_id);
    const newest = notes.reduce<(typeof notes)[number] | null>((a, n) => (!a || n.created_at >= a.created_at ? n : a), null);
    if (!newest) continue;
    out.set(newest.id, [...(out.get(newest.id) ?? []), r]);
  }
  return out;
}

/** What the timeline says about a card that cannot be pushed from here (null: it can). */
export function blockedReason(r: Pick<ExtReadyPush, "state" | "branch">): string | null {
  switch (r.state) {
    case "ready": return null;
    case "pushed": return `Home already pushes ${r.branch} to this pull request.`;
    case "moved": return `${r.branch} moved since the agent prepared this push: check its changes in Launchpad.`;
    case "gone": return `Home no longer has ${r.branch}.`;
    default: return "This push is not available.";
  }
}
