// Shared primitives for dedicated background requests. Route-specific trust checks stay with each route.
import type { ItemAction } from "@/gen/ItemAction";
import { RequestError, type SuggestionEdit } from "../shared/messages.ts";

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


const MAX_TEXT = 65_536;
const line = (v: unknown) => (Number.isInteger(v) && (v as number) > 0 ? (v as number) : null);

/** Pure: the edit a page sent, rebuilt field by field, or an error. The text stays as typed (Markdown and ```suggestion
 * blocks untouched); a review keeps its event and follow-up; `comments` (a review's line comments with their new text,
 * the ones left out removed) is absent for a comment, so Home keeps them. Places are checked again on Home. */
export function checkedEdit(raw: unknown): SuggestionEdit {
  const a = (raw ?? {}) as Record<string, unknown>;
  const text = (v: unknown, what: string) => {
    if (typeof v !== "string" || !v.trim() || v.length > MAX_TEXT) throw new RequestError(`${what} holds 1–65536 characters`);
    return v;
  };
  const body = text(a.body, "A comment");
  const event = a.event ?? "COMMENT";
  if (event !== "COMMENT" && event !== "APPROVE" && event !== "REQUEST_CHANGES") throw new RequestError("Not a review event");
  const merge = a.merge ?? null;
  if (merge !== null && merge !== "merge" && merge !== "squash" && merge !== "rebase") throw new RequestError("Not a merge method");
  const out: SuggestionEdit = { body, event, merge, run_ci: a.run_ci === true };
  if (a.comments !== undefined) {
    if (!Array.isArray(a.comments) || a.comments.length > 200) throw new RequestError("Not a list of line comments");
    out.comments = a.comments.map((c: Record<string, unknown>) => {
      const at = line(c?.line);
      if (typeof c?.path !== "string" || !c.path || c.path.length > 1024 || at === null) throw new RequestError("Not a line comment's place");
      if (c.side !== "RIGHT" && c.side !== "LEFT") throw new RequestError("Not a side of the diff");
      const start = c.start_line === null || c.start_line === undefined ? null : line(c.start_line);
      if (c.start_line != null && start === null) throw new RequestError("Not a line comment's place");
      return { path: c.path, line: at, side: c.side, start_line: start, body: text(c.body, `The line comment on ${c.path}:${at}`) };
    });
  }
  return out;
}
