import type { Session } from "@/gen/Session";
/** "Browser" for browser-only tasks, "<project> · Browser" for browser tasks that work in a project. */
export const sessionPlace = (s: Session, project: string) => s.launch_spec.target.kind !== "browser" ? project : s.launch_spec.target.project_id ? `${project} · Browser` : "Browser";
export const isDone = (s: Session) => s.status === "waiting";
/** Archived Done sessions still satisfy scheduled agent dependencies; an open question does not. */
export const dependencyDone = (s: Session) => (isDone(s) && !s.open_question) || (s.status === "closed" && s.archived_done);
export const isRunning = (s: Session) => ["starting", "working", "background"].includes(s.status);

export const isQuotaPaused = (s: Session) => s.status === "quota_paused";
export const isStopped = (s: Session) => s.status === "stopped";
export const isUnseen = (s: Session) => s.seen_at < s.status_since;
/** The shared "Needs you" predicate (same as `Session::needs_you` in Rust): an open question, or Done, Failed or Interrupted not seen since its status changed. */
export const needsYou = (s: Session) => (isDone(s) && (s.open_question || isUnseen(s))) || ((s.status === "error" || s.status === "interrupted") && isUnseen(s));
/** Group order for session pickers: needs you, running, done, stopped or failed, archived. */
const pickerGroup = (s: Session) => needsYou(s) ? 0 : isRunning(s) || isQuotaPaused(s) || s.status === "in_terminal" ? 1 : isDone(s) ? 2 : s.status === "closed" ? 4 : 3;
/** Sorts sessions by group, then by latest status change first. */
export const byPickerOrder = (a: Session, b: Session) => pickerGroup(a) - pickerGroup(b) || b.status_since - a.status_since;
