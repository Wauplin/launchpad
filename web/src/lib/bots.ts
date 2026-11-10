// Bots: named, persistent agents of a workspace, stored by Home as files (BOT.md, bot.toml). A bot's thread is
// the chain of sessions whose launch spec names it; the newest one that is not archived is its current session.
// Ask Launchpad and the Librarian are two of them: Home creates each once per workspace.

import { useQuery } from "@tanstack/react-query";
import type { Bot } from "@/gen/Bot";
import type { Session } from "@/gen/Session";
import { api } from "./api";
import { isQuotaPaused, isRunning, needsYou } from "./sessionStatus";

/** Ready-made bots the New bot form starts from (Ask Launchpad, the Librarian). */
export type BotTemplate = { id: string; name: string; instructions: string; starters?: string[] };
export const useBotTemplates = () => useQuery({ queryKey: ["bot-templates"], queryFn: () => api.get<BotTemplate[]>("/api/bots/templates"), staleTime: Infinity, retry: false });

/** The profile a bot runs with: its own, or (none in bot.toml) the first subscription profile, else the first. */
export function botProfile<P extends { id: string; billing: string }>(bot: Bot, profiles: P[]): P | null {
  if (bot.settings.profile) return profiles.find((p) => p.id === bot.settings.profile) ?? null;
  return profiles.find((p) => p.billing === "subscription") ?? profiles[0] ?? null;
}

export const useBots = () => useQuery({ queryKey: ["bots"], queryFn: () => api.get<Bot[]>("/api/bots"), staleTime: 30_000, refetchInterval: 60_000, retry: false });

/** The workspace's Librarian (Home seeds it as `librarian`), which tends the knowledge base; null once the owner deleted it. */
export const librarianOf = (bots: Bot[] | undefined, ws: string | null) => bots?.find((b) => b.workspace_id === ws && b.slug === "librarian") ?? null;

export const botOf = (s: Session) => s.launch_spec.bot?.id ?? null;

/** The bot's sessions, oldest first. */
export const botThread = (sessions: Session[], id: string) => sessions.filter((s) => botOf(s) === id).sort((a, b) => a.started_at - b.started_at);

export const currentSession = (thread: Session[]) => [...thread].reverse().find((s) => s.status !== "closed") ?? null;

/** Why the next message starts a new session (the same rule as Home's `bots::rotation`), or null. `profile` is
 * the bot's profile: its model and effort apply when bot.toml names none. */
export function rotationReason(bot: Bot, current: Session | null, profile: { id: string; model: string | null; effort: string | null } | null): string | null {
  if (!current) return null;
  if (current.launch_spec.profile !== (profile?.id ?? null)) return "The bot's profile changed";
  // Without a folder, each session runs in its own temporary folder.
  const folder = current.launch_spec.target.kind === "temp" ? null : current.project_id;
  if (folder !== (bot.settings.folder ?? null)) return "The bot's folder changed";
  if ((current.launch_spec.model ?? null) !== (bot.settings.model ?? profile?.model ?? null)) return "The bot's model changed";
  if ((current.launch_spec.effort ?? null) !== (bot.settings.effort ?? profile?.effort ?? null)) return "The bot's effort changed";
  // The mode bot.toml gave the session at start: one the owner set on the session in Agents keeps the thread.
  if ((current.launch_spec.bot?.permission_mode ?? current.launch_spec.permission_mode) !== bot.settings.permission_mode) return "The bot's permission mode changed";
  return null;
}

/** The current session runs an older BOT.md: Home sends the new one with the next message (`bots::with_new_instructions`). */
export const instructionsUpdated = (bot: Bot, current: Session | null) => {
  const v = current?.launch_spec.bot?.version;
  return !!v && v !== bot.instructions_version;
};

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

/** The bot the top-bar Ask field talks to by default: the oldest bot of the workspace with admin Launchpad CLI
 * access (Ask Launchpad unless you changed it), else the oldest bot. Any bot can take that role. */
export function askTarget(bots: Bot[], workspaceId: string | null | undefined): Bot | null {
  const mine = bots.filter((b) => b.workspace_id === workspaceId).sort((a, b) => a.created_at - b.created_at);
  return mine.find((b) => b.settings.launchpad === "admin") ?? mine[0] ?? null;
}
