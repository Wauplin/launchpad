import type { Session } from "../gen/Session";

/** The thread of sessions from before bots had several threads. */
const LEGACY_THREAD = "main";

/** The thread of a bot launch. */
export const launchThread = (b: { thread?: string | null }) => b.thread || LEGACY_THREAD;

/** The thread of a bot session. */
export const threadOf = (s: Session) => launchThread(s.launch_spec.bot ?? {});

/** The open session of each thread of a bot's sessions (each thread's newest, unless archived), by thread. */
export function threadCurrents(sessions: Session[]): Map<string, Session> {
  const out = new Map<string, Session>();
  for (const s of [...sessions].sort((a, b) => a.started_at - b.started_at)) out.set(threadOf(s), s);
  for (const [t, s] of out) if (s.status === "closed") out.delete(t);
  return out;
}
