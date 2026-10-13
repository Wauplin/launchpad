import type { Session } from "@/gen/Session";
export const isDone = (s: Session) => s.status === "waiting";
/** Archived Done sessions still satisfy scheduled agent dependencies. */
export const dependencyDone = (s: Session) => isDone(s) || (s.status === "closed" && s.archived_done);
export const isRunning = (s: Session) => ["starting", "working", "background"].includes(s.status);

export const isQuotaPaused = (s: Session) => s.status === "quota_paused";
export const isStopped = (s: Session) => s.status === "stopped";
export const isUnseen = (s: Session) => s.seen_at < s.status_since;
/** The shared "Needs you" predicate (same as `Session::needs_you` in Rust): an open question, or Done, Failed or Interrupted not seen since its status changed. */
export const needsYou = (s: Session) => (isDone(s) && (s.open_question || isUnseen(s))) || ((s.status === "error" || s.status === "interrupted") && isUnseen(s));
