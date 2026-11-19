// Home's maintenance during a planned update (`launchpad_core::maintenance`): every window shows it, and a write Home
// refused for it is a definite "not done, try later" (HTTP 503 with `retryable` and `maintenance`), never an
// uncertain delivery: the gate answers before anything runs.

import type { Maintenance } from "@/gen/Maintenance";

/** The maintenance a refused write carries (an `ApiError`'s body), or null. */
export function refusedFor(error: unknown): Maintenance | null {
  const e = error as { status?: number; body?: Record<string, unknown> | null } | null;
  if (!e || e.status !== 503 || !e.body || e.body.retryable !== true) return null;
  const m = e.body.maintenance as Maintenance | undefined;
  return m && typeof m === "object" && typeof m.phase === "string" ? m : null;
}

/** Minutes until a refused write is worth trying again. */
export function retryMinutes(m: Maintenance): number {
  return Math.max(1, Math.ceil(m.retry_after / 60));
}

/** What a refused write says in a toast: nothing was changed, the typed text stays. */
export function refusalText(m: Maintenance): string {
  return `${m.reason}: Home accepts no changes until it runs again. Nothing was sent or changed, and your text is kept: try again in about ${retryMinutes(m)} min.`;
}

/** The banner of every window: from the last status received and whether the live connection works. During the
 * replacement the connection drops (the old Home stops); the banner then says Home is restarting, and the first
 * status of the new Home (refetched on reconnect) clears it. */
export function bannerText(m: Maintenance | null | undefined, online: boolean): string | null {
  if (!m) return null;
  const what = m.phase === "draining" ? "is finishing the requests in progress" : m.phase === "checkpointing" ? "is saving its final backup" : online ? "is building the new version" : "is restarting with the new version";
  return `${m.reason}. Home ${what}: changes are paused, and drafts are kept. Agents keep working on their runners.`;
}
