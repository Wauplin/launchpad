// Bots: named, persistent agents of a workspace, stored by Home as files (BOT.md, bot.toml). A bot has several
// threads (its parallel conversations): a thread is one session whose launch spec names the bot and the thread
// (`launch_spec.bot.thread`; empty in sessions from before threads: the legacy thread `main`). Threads from before
// a thread was one session may hold several: only the newest counts.
// Ask Launchpad and the Librarian are two of them: Home creates each once per workspace.

import { threadOf, threadCurrents } from "./botThread";
import { useQuery, type QueryClient } from "@tanstack/react-query";
import type { Bot } from "@/gen/Bot";
import type { BotSend } from "@/gen/BotSend";
import type { BotThread } from "@/gen/BotThread";
import type { Session } from "@/gen/Session";
import { api } from "./api";
import { byName } from "./channelRows";
import { isQuotaPaused, isRunning, needsYou } from "./sessionStatus";
import { initialOf } from "./identity";

/** The owner's bot messages this window sent: a chat follows only its own message into a new thread. */
export const sentHere = new Set<string>();

/** Shows a message Home accepted (`bot.message` with `accept`) at once, as a bubble in its thread's chat, before
 * Home's list of accepted messages (`GET /api/bots/{id}/sends`) refreshes. */
export function showAccepted(qc: QueryClient, sent: BotSend) {
  sentHere.add(sent.id);
  qc.setQueryData<BotSend[]>(["bot-sends", sent.bot_id], (old) => (old ?? []).some((m) => m.id === sent.id) ? old : [...(old ?? []), sent]);
  void qc.invalidateQueries({ queryKey: ["bot-sends", sent.bot_id] });
}

/** Ready-made bots the New bot form starts from (Ask Launchpad, the Librarian). */
export type BotTemplate = { id: string; name: string; instructions: string; starters?: string[] };
export const useBotTemplates = () => useQuery({ queryKey: ["bot-templates"], queryFn: () => api.get<BotTemplate[]>("/api/bots/templates"), staleTime: Infinity, retry: false });

/** The profile a bot runs with: its own, or (none in bot.toml) the first subscription profile, else the first. */
export function botProfile<P extends { id: string; billing: string }>(bot: Bot, profiles: P[]): P | null {
  if (bot.settings.profile) return profiles.find((p) => p.id === bot.settings.profile) ?? null;
  if (bot.settings.profiles?.length) return profiles.find((p) => p.id === bot.settings.profiles?.[0]) ?? null;
  return profiles.find((p) => p.billing === "subscription") ?? profiles[0] ?? null;
}

/** Every list of bots shows them in alphabetical order of their names (ignoring case), so the query sorts once. */
const botsByName = (bots: Bot[]) => byName(bots, (b) => b.settings.name);
export const useBots = () => useQuery({ queryKey: ["bots"], queryFn: () => api.get<Bot[]>("/api/bots"), select: botsByName, staleTime: 30_000, refetchInterval: 60_000, retry: false });

/** The workspace's Librarian (Home seeds it as `librarian`), which tends the knowledge base; null once the owner deleted it. */
export const librarianOf = (bots: Bot[] | undefined, ws: string | null) => bots?.find((b) => b.workspace_id === ws && b.slug === "librarian") ?? null;

export const botOf = (s: Session) => s.launch_spec.bot?.id ?? null;

export { launchThread, threadOf, threadCurrents } from "./botThread";

/** The bot's sessions, oldest first; with `thread`, that thread's session (its newest) alone. */
export const botThread = (sessions: Session[], id: string, thread?: string) => {
  const list = sessions.filter((s) => botOf(s) === id && (thread === undefined || threadOf(s) === thread)).sort((a, b) => a.started_at - b.started_at);
  return thread === undefined ? list : list.slice(-1);
};

/** The thread the bot page opens on this device: the last one the owner looked at (browser storage; the demo keeps
 * nothing). */
const LAST_THREAD_KEY = (bot: string) => `launchpad.bot-thread.${bot}`;
export const lastThread = (bot: string) => (window.__lpDemo ? null : localStorage.getItem(LAST_THREAD_KEY(bot)));
export const rememberThread = (bot: string, thread: string) => { if (!window.__lpDemo) localStorage.setItem(LAST_THREAD_KEY(bot), thread); };

/** The thread to open when none is named: the last one seen on this device while it is still open, else the one
 * whose session the owner saw last, else the most recent; null when the bot has none (a new thread). */
export function landingThread(bot: string, threads: BotThread[], sessions: Session[]): string | null {
  const open = new Set(threads.map((t) => t.id));
  const remembered = lastThread(bot);
  if (remembered && open.has(remembered)) return remembered;
  const mine = sessions.filter((s) => botOf(s) === bot && open.has(threadOf(s)));
  const seen = mine.filter((s) => s.seen_at > 0).sort((a, b) => b.seen_at - a.seen_at)[0];
  if (seen) return threadOf(seen);
  const recent = mine.sort((a, b) => b.status_since - a.status_since)[0];
  return recent ? threadOf(recent) : threads[0]?.id ?? null;
}

/** Refreshes the thread lists after a change made here (a bot's lists, every bot's without `bot`) and the Chats
 * list's threads; the `bots`/`threads` bus event does the same for other windows. */
export const refreshThreads = (qc: QueryClient, bot?: string) => Promise.all([qc.invalidateQueries({ queryKey: bot ? ["bot-threads", bot] : ["bot-threads"] }), qc.invalidateQueries({ queryKey: ["chat-threads"] })]);

/** The bot's threads (the newest first), archived ones with `archived`. */
export const useBotThreads = (id: string | null | undefined, archived = false) => useQuery({ queryKey: ["bot-threads", id, archived], queryFn: () => api.get<BotThread[]>(`/api/bots/${id}/threads${archived ? "?archived=true" : ""}`), enabled: !!id, staleTime: 15_000, retry: false });

/** The thread a `?thread=` value names: a thread ID, or a routing key such as `channel:<id>` (links from a channel). */
export const findThread = (threads: BotThread[] | undefined, value: string | undefined) => value ? threads?.find((t) => t.id === value || t.key === value) ?? null : null;

export const currentSession = (thread: Session[]) => [...thread].reverse().find((s) => s.status !== "closed") ?? null;

/** Why the next message continues in a new thread (the same rule as Home's `bots::rotation`), or null. `profile` is
 * the bot's profile: its model and effort apply when bot.toml names none. */
export function rotationReason(bot: Bot, current: Session | null, profile: { id: string; model: string | null; effort: string | null } | null): string | null {
  if (!current) return null;
  if (current.status === "closed") return "This chat is archived";
  if (!bot.settings.profile && bot.settings.profiles?.includes(current.launch_spec.profile ?? "")) {
    // Keep the selected fallback for this session, including its model defaults.
    profile = { id: current.launch_spec.profile!, model: current.launch_spec.model ?? null, effort: current.launch_spec.effort ?? null };
  }
  if (current.launch_spec.profile !== (profile?.id ?? null)) return "The bot's profile changed";
  // Without a folder, each session runs in its own temporary folder.
  const folder = current.launch_spec.target.kind === "temp" ? null : current.project_id;
  // A plain folder (no longer used by bots): its sessions run in a temporary folder.
  if (folder !== (bot.folder_unused ? null : bot.settings.folder ?? null)) return "The bot's folder changed";
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

/** The state of a bot over all its threads (`sessions`: every session of the bot): needs you when one thread does,
 * else working when one works, else quota paused. */
export function botState(sessions: Session[]): BotState {
  const open = [...threadCurrents(sessions).values()];
  if (!open.length) return sessions.length ? "idle" : "new";
  if (open.some(needsYou)) return "needs";
  if (open.some(isRunning)) return "running";
  if (open.some(isQuotaPaused)) return "paused";
  return "idle";
}

/** The state of one thread (its session). */
export function threadState(thread: Session[]): BotState {
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

export const botInitial = (name: string) => initialOf(name);

/** The bot the top-bar Ask field talks to by default: the oldest bot of the workspace with admin Launchpad CLI
 * access (Ask Launchpad unless you changed it), else the oldest bot. Any bot can take that role. */
export function askTarget(bots: Bot[], workspaceId: string | null | undefined): Bot | null {
  const mine = bots.filter((b) => b.workspace_id === workspaceId).sort((a, b) => a.created_at - b.created_at);
  return mine.find((b) => b.settings.launchpad === "admin") ?? mine[0] ?? null;
}
