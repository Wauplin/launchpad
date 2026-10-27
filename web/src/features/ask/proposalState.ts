import type { Proposal } from "../../gen/Proposal";

export function isWaiting(p: Pick<Proposal, "state">): boolean {
  return p.state === "proposed" || p.state === "failed" || p.state === "uncertain";
}

/** Keep a linked successor visible even when it falls outside the recent history. */
export function visibleProposals<T extends Pick<Proposal, "id" | "state">>(proposals: T[], waitingOnly: boolean, reveal: string | null): T[] {
  if (waitingOnly) return proposals.filter(isWaiting);
  return proposals.filter((p, i) => i < 50 || p.id === reveal);
}
