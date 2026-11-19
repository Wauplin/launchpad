// Active threads (the phone's Chats tab, the Bots page and the sidebar): the open threads of every bot of the workspace
// in one list, newest update first. Active = not archived, and live or waiting for you (working, a question or an
// unseen turn, sleeping, quota paused, failed or interrupted) or updated in the last 24 hours. Pure, so it is tested
// without the app.

import type { Bot } from "../gen/Bot";
import type { BotThread } from "../gen/BotThread";
import type { Session } from "../gen/Session";
import { threadOf } from "./botThread.ts";
import { isQuotaPaused, isRunning, isSleeping, needsYou } from "./sessionStatus.ts";

/** How recent a thread with nothing live must be to count as active. */
export const ACTIVE_WINDOW_MS = 24 * 60 * 60 * 1000;
/** Rows shown before Show all. */
export const ACTIVE_SHOWN = 6;

export type ThreadTone = "running" | "needs" | "done" | "failed" | "idle";
export type ActiveThread = { bot: Bot; thread: string; title: string; session: Session; last: number; attention: boolean; label: string; tone: ThreadTone };

const failed = (s: Session) => s.status === "error" || s.status === "interrupted";

/** Live or waiting for you: never hidden behind the cap by newer finished threads. */
export const isAttention = (s: Session) => isRunning(s) || needsYou(s) || isSleeping(s) || isQuotaPaused(s) || failed(s);

/** A short status, in the words and colors of the session status pill (`statusInfo`). */
export function threadStatus(s: Session): { label: string; tone: ThreadTone } {
  switch (s.status) {
    case "starting": return { label: "Starting", tone: "running" };
    case "working": return { label: "Working", tone: "running" };
    case "background": return { label: "Background", tone: "running" };
    case "waiting":
      if (s.open_question) return { label: "Question", tone: "needs" };
      if (s.sleep) return { label: "Sleeping", tone: "idle" };
      return { label: "Done", tone: "done" };
    case "quota_paused": return { label: "Quota paused", tone: "needs" };
    case "interrupted": return { label: "Interrupted", tone: "failed" };
    case "error": return s.capacity_retry_at != null ? { label: "Retry pending", tone: "idle" } : { label: "Failed", tone: "failed" };
    case "stopped": return { label: "Stopped", tone: "idle" };
    case "in_terminal": return { label: "In terminal", tone: "idle" };
    case "closed": return { label: "Archived", tone: "idle" };
  }
}

/** The active threads of the workspace's bots, newest update first. `threads` holds each bot's open threads (Home's
 * list, for their titles) when loaded: a thread missing from a loaded list is archived. */
export function activeThreads(bots: Bot[], sessions: Session[], threads: Map<string, BotThread[] | undefined>, workspaceId: string | null | undefined, now: number): ActiveThread[] {
  const mine = new Map(bots.filter((b) => b.workspace_id === workspaceId).map((b) => [b.id, b]));
  // Each thread's newest session.
  const current = new Map<string, Session>();
  for (const s of [...sessions].sort((a, b) => a.started_at - b.started_at)) {
    const bot = s.launch_spec.bot?.id;
    if (bot && mine.has(bot)) current.set(`${bot}\n${threadOf(s)}`, s);
  }
  const rows: ActiveThread[] = [];
  for (const s of current.values()) {
    if (s.status === "closed") continue;
    const bot = mine.get(s.launch_spec.bot!.id)!;
    const thread = threadOf(s);
    const list = threads.get(bot.id);
    const record = list?.find((t) => t.id === thread);
    if (list && !record) continue;
    const attention = isAttention(s);
    if (!attention && now - s.status_since > ACTIVE_WINDOW_MS) continue;
    rows.push({ bot, thread, title: record?.title || s.title, session: s, last: s.status_since, attention, ...threadStatus(s) });
  }
  return rows.sort((a, b) => b.last - a.last || a.bot.settings.name.localeCompare(b.bot.settings.name));
}

/** At most `max` rows in their order, keeping the threads that need attention before newer finished ones. */
export function capThreads(rows: ActiveThread[], max = ACTIVE_SHOWN): ActiveThread[] {
  if (rows.length <= max) return rows;
  const attention = rows.filter((r) => r.attention).slice(0, max);
  const keep = new Set([...attention, ...rows.filter((r) => !r.attention).slice(0, max - attention.length)]);
  return rows.filter((r) => keep.has(r));
}
