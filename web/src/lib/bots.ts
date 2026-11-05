// Bots: named, persistent agents of a workspace, stored by Home as files (BOT.md, bot.toml). A bot's thread is
// the chain of sessions whose launch spec names it; the newest one that is not archived is its current session.

import { useQuery } from "@tanstack/react-query";
import type { AskBot } from "@/gen/AskBot";
import type { Bot } from "@/gen/Bot";
import type { Session } from "@/gen/Session";
import { api } from "./api";
import { isQuotaPaused, isRunning, needsYou } from "./sessionStatus";

/** Ask Launchpad, the built-in bot: its BOT.md, the default and the built-in rules (`/api/assistant/bot`). */
export const ASK_BOT_ID = "ask-launchpad";
export const useAskBot = () => useQuery({ queryKey: ["ask-bot"], queryFn: () => api.get<AskBot>("/api/assistant/bot"), staleTime: 30_000, retry: false });

export const useBots = () => useQuery({ queryKey: ["bots"], queryFn: () => api.get<Bot[]>("/api/bots"), staleTime: 30_000, refetchInterval: 60_000, retry: false });

export const botOf = (s: Session) => s.launch_spec.bot?.id ?? null;

/** The bot's sessions, oldest first. */
export const botThread = (sessions: Session[], id: string) => sessions.filter((s) => botOf(s) === id).sort((a, b) => a.started_at - b.started_at);

export const currentSession = (thread: Session[]) => [...thread].reverse().find((s) => s.status !== "closed") ?? null;

/** Why the next message starts a new session (the same rule as Home's `bots::rotation`), or null. */
export function rotationReason(bot: Bot, current: Session | null): string | null {
  if (!current) return null;
  if (current.launch_spec.profile !== (bot.settings.profile ?? null)) return "The bot's profile changed";
  if (current.project_id !== bot.settings.folder) return "The bot's folder changed";
  return null;
}

export type BotState = "needs" | "running" | "paused" | "idle" | "new";

export function botState(thread: Session[]): BotState {
  const s = currentSession(thread);
  if (!s) return thread.length ? "idle" : "new";
  if (needsYou(s)) return "needs";
  if (isRunning(s)) return "running";
  if (isQuotaPaused(s)) return "paused";
  return "idle";
}

/** One line for lists: the first paragraph of BOT.md, without its title. */
export function botJob(bot: Bot): string {
  const lines = bot.instructions.split("\n").map((l) => l.trim());
  const body = lines.filter((l) => l && !l.startsWith("#"));
  return body[0] ?? "";
}

export const botInitial = (name: string) => (name.trim()[0] ?? "B").toUpperCase();
