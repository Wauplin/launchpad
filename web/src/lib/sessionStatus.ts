import type { Session } from "@/gen/Session";
export const isDone = (s: Session) => s.status === "waiting";
export const isRunning = (s: Session) => ["starting", "working", "background"].includes(s.status);
