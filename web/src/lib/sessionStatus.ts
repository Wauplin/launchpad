import type { Session } from "@/gen/Session";
import type { SessionGroup } from "@/gen/SessionGroup";
import type { Sleep } from "@/gen/Sleep";
import { when } from "./format.ts";
export const isDone = (s: Session) => s.status === "waiting";
/** Archived Done sessions still satisfy scheduled agent dependencies; an open question does not. */
export const dependencyDone = (s: Session) => (isDone(s) && !s.open_question && !s.sleep) || (s.status === "closed" && s.archived_done);
export const isRunning = (s: Session) => ["starting", "working", "background"].includes(s.status);

export const isQuotaPaused = (s: Session) => s.status === "quota_paused";
export const isStopped = (s: Session) => s.status === "stopped";
export const isUnseen = (s: Session) => s.seen_at < s.status_since;
/** Sleeping (same as `Session::sleeping` in Rust): Done, nothing asked, and Home wakes it later (`launchpad sleep`). */
export const isSleeping = (s: Session) => isDone(s) && !s.open_question && !!s.sleep;
/** What a sleeping agent waits for, in words: "until 15:00", "until “Fix CI” (1a2b3c4d) is Done, at the latest today 17:00". */
export function sleepUntil(z: Sleep): string {
  const parts: string[] = [];
  if (z.waits) parts.push(`until ${z.waits}`);
  if (z.until != null) parts.push(z.waits ? `not before ${when(z.until)}` : `until ${when(z.until)}`);
  if (z.deadline != null) parts.push(`at the latest ${when(z.deadline)}`);
  return parts.join(", ");
}
/** Sort key of sleeping agents: the soonest known wake-up first. */
export const sleepOrder = (s: Session) => s.sleep?.until ?? s.sleep?.deadline ?? Number.MAX_SAFE_INTEGER;
/** The shared "Needs you" predicate (same as `Session::needs_you` in Rust): an open question, or Done (not sleeping), Failed or Interrupted not seen since its status changed. */
export const needsYou = (s: Session) => (isDone(s) && (s.open_question || (isUnseen(s) && !isSleeping(s)))) || ((s.status === "error" || s.status === "interrupted") && isUnseen(s));
/** A session's group in lists: Home's `group` (`Session::group` in Rust) on light records and bus events; the same
 * rule here for full records and the demo. Both are checked against `web/tests/session-groups/cases.json`. */
export function sessionGroup(s: Session): SessionGroup {
  if (s.group) return s.group;
  if (s.status === "closed") return "archived";
  if (needsYou(s)) return "needs_you";
  if (isSleeping(s)) return "sleeping";
  switch (s.status) {
    case "waiting": return "done";
    case "starting": case "working": case "background": return "running";
    case "quota_paused": return "quota_paused";
    case "stopped": return "stopped";
    case "interrupted": return "interrupted";
    case "error": return "failed";
    case "in_terminal": return "in_terminal";
  }
}
const PICKER: Record<SessionGroup, number> = { needs_you: 0, running: 1, quota_paused: 1, in_terminal: 1, done: 2, sleeping: 2, stopped: 3, interrupted: 3, failed: 3, archived: 4 };
/** Group order for session pickers: needs you, running, done, stopped or failed, archived. */
const pickerGroup = (s: Session) => PICKER[sessionGroup(s)];
/** Sorts sessions by group, then by latest status change first. */
export const byPickerOrder = (a: Session, b: Session) => pickerGroup(a) - pickerGroup(b) || b.status_since - a.status_since;
