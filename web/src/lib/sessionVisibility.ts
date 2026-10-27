import type { Session } from "../gen/Session.ts";

/** Assistant threads are reachable through Ask, direct IDs and explicit opt-in lists only. */
export const normalSessions = (sessions: Session[]) => sessions.filter((s) => s.launch_spec.target.kind !== "assistant");
