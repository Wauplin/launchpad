// Shared primitives for dedicated background requests. Route-specific trust checks stay with each route.
import type { ItemAction } from "@/gen/ItemAction";
import { RequestError } from "../shared/messages.ts";

export function id(v: unknown, what: string): string {
  if (typeof v !== "string" || !/^[A-Za-z0-9_-]{1,80}$/.test(v)) throw new RequestError(`Not a ${what} ID`);
  return v;
}

export function workspace(ws: string | null): string {
  if (!ws) throw new RequestError("Choose a workspace in the Launchpad panel first");
  return ws;
}

const LOGIN = /^[A-Za-z0-9_-][A-Za-z0-9_.-]{0,99}$/;

/** Pure: the actions a page sent, rebuilt field by field (only known kinds and values), or an error. */
export function checkedActions(raw: unknown): ItemAction[] {
  if (!Array.isArray(raw) || raw.length > 6) throw new RequestError("Not a list of actions");
  return raw.map((a): ItemAction => {
    const kind = (a as { kind?: unknown })?.kind;
    switch (kind) {
      case "reopen": case "ready_for_review": case "convert_to_draft": case "update_branch": return { kind };
      case "close": {
        const { reason, duplicate_of } = a as { reason?: unknown; duplicate_of?: unknown };
        if (reason !== null && reason !== undefined && reason !== "completed" && reason !== "not_planned" && reason !== "duplicate") throw new RequestError("Not a close reason");
        if (duplicate_of !== null && duplicate_of !== undefined && !(Number.isInteger(duplicate_of) && (duplicate_of as number) > 0)) throw new RequestError("Not an issue number");
        return { kind, reason: (reason ?? null) as "completed" | "not_planned" | "duplicate" | null, duplicate_of: (duplicate_of ?? null) as number | null };
      }
      case "request_reviewers": {
        const names = (v: unknown) => { if (!Array.isArray(v) || !v.every((x) => typeof x === "string" && LOGIN.test(x))) throw new RequestError("Not a list of reviewers"); return v as string[]; };
        const { reviewers, team_reviewers } = a as { reviewers?: unknown; team_reviewers?: unknown };
        return { kind, reviewers: names(reviewers ?? []), team_reviewers: names(team_reviewers ?? []) };
      }
      default: throw new RequestError("Not an action");
    }
  });
}

