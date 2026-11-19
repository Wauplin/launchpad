// A bot's threads: its parallel conversations, each one session (the owner's threads, one per GitHub item,
// channel, hook, agent or Telegram, one per routine run), shown as chats. The bot page shows one thread at a time
// (`?thread=`, the only place that state lives); the Chats list is the navigation, so the chat has no thread bar.
// Archive is one click, like an agent's: on the opened chat (`ArchiveThreadButton`) and in ⋯ of each row of the Chats
// list (`useArchiveThread`). Archiving the chat shown goes back to the Chats landing, never to another chat. Archived
// chats come back from the list's Archived section (⋯ → Unarchive).

import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useMemo, useState } from "react";
import { create } from "zustand";
import type { Bot } from "@/gen/Bot";
import type { BotThread } from "@/gen/BotThread";
import type { Session } from "@/gen/Session";
import { Icon } from "@/components/Icon";
import { toast, toastError } from "@/components/Toast";
import { IconButton } from "@/components/ui";
import { runAction } from "@/lib/actions";
import { botThread, threadState, useBotThreads, type BotState } from "@/lib/bots";
import { usePhone } from "@/lib/phone";
import { isRunning } from "@/lib/sessionStatus";
import { phoneBack } from "@/mobile/back";

/** The `?thread=` value of a thread the owner starts with their next message. */
export const NEW_THREAD = "new";

/** A thread with its state and last activity, from the bot's sessions. */
export type ThreadRow = { t: BotThread; state: BotState; last: number };

/** The threads of a bot with their states, by last activity, newest first. */
export function useThreadRows(bot: Bot, sessions: Session[]): ThreadRow[] {
  const threads = useBotThreads(bot.id).data;
  return useMemo(() => {
    const rows = (threads ?? []).map((t): ThreadRow => {
      const mine = botThread(sessions, bot.id, t.id);
      return { t, state: threadState(mine), last: mine.at(-1)?.status_since ?? t.created_at };
    });
    return rows.sort((a, b) => b.last - a.last);
  }, [threads, sessions, bot.id]);
}

/** The thread the bot page shows now (null elsewhere), so an Archive from the sidebar knows to leave it. */
export const useShownThread = create<{ shown: { bot: string; thread: string; conversation: string | null } | null }>(() => ({ shown: null }));

/** Archive a chat (every thread of its conversation, `bot.thread-archive`) in one click, then refresh the lists.
 * When the bot page shows it, the page goes to the Chats landing (the phone back to the list): it never opens another
 * chat on its own. */
export function useArchiveThread() {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const phone = usePhone();
  const [busy, setBusy] = useState<string | null>(null);
  const archive = async (bot: string, thread: string, conversation: string | null) => {
    setBusy(`${bot}:${thread}`);
    try {
      await runAction("bot.thread-archive", { id: bot, thread }, { archived: true });
      const shown = useShownThread.getState().shown;
      const leaving = !!shown && ((shown.bot === bot && shown.thread === thread) || (!!conversation && shown.conversation === conversation));
      if (leaving) { if (phone) phoneBack("/chats"); else void navigate({ to: "/chats", search: {} as never }); }
      toast("Chat archived: it stays under Archived in Chats");
      await Promise.all([qc.invalidateQueries({ queryKey: ["bot-threads"] }), qc.invalidateQueries({ queryKey: ["sessions"] })]);
    } catch (e) { toastError(e); }
    finally { setBusy(null); }
  };
  return { archive, busy: (bot: string, thread: string) => busy === `${bot}:${thread}` };
}

/** Archive of the chat shown, one click: a primary button in the chat's header on the laptop, an icon of the chat's
 * bar on the phone. Disabled while the chat works. */
export function ArchiveThreadButton({ bot, thread, sessions }: { bot: Bot; thread: BotThread; sessions: Session[] }) {
  const phone = usePhone();
  const { archive, busy } = useArchiveThread();
  const running = sessions.some(isRunning);
  const working = busy(bot.id, thread.id);
  const title = running ? "Stop its turn first to archive this chat" : `Archive “${thread.title}”: it stays under Archived in Chats`;
  const run = () => void archive(bot.id, thread.id, thread.conversation ?? null);
  if (phone) return <IconButton icon="archive" label={running ? title : "Archive this chat"} disabled={running || working} data-tour="bots.archive-thread" onClick={run} />;
  return <button type="button" className="btn btn-sm btn-primary flex-none" disabled={running || working} title={title} data-tour="bots.archive-thread" onClick={run}><Icon name="archive" size={12} />{working ? "Archiving…" : "Archive"}</button>;
}
