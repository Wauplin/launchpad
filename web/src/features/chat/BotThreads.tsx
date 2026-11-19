// A bot's threads: its parallel conversations, each one session (the owner's threads, one per GitHub item,
// channel, hook, agent or Telegram, one per routine run). The bot page shows one thread at a time (`?thread=`, the
// only place that state lives); the sidebar's Active threads and the Bots page are the navigation, so the chat has no
// thread bar. Archive is one click, like an agent's: on the opened thread (`ArchiveThreadButton`) and on each Active
// threads row of the sidebar (`useArchiveThread`). Archiving the thread shown goes back to the Bots page, never to
// another thread. Archived threads come back from ⋯ → Archived threads (`ArchivedThreads`).

import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useMemo, useState } from "react";
import { create } from "zustand";
import type { Bot } from "@/gen/Bot";
import type { BotThread } from "@/gen/BotThread";
import type { Session } from "@/gen/Session";
import { ActionButton } from "@/components/ActionButton";
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

/** Archive a thread (every thread of its conversation, `bot.thread-archive`) in one click, then refresh the lists.
 * When the bot page shows it, the page goes to the Bots page (the phone back to Chats): it never opens another
 * thread on its own. */
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
      if (leaving) { if (phone) phoneBack("/chats"); else void navigate({ to: "/bots", search: {} as never }); }
      toast("Thread archived");
      await Promise.all([qc.invalidateQueries({ queryKey: ["bot-threads"] }), qc.invalidateQueries({ queryKey: ["sessions"] })]);
    } catch (e) { toastError(e); }
    finally { setBusy(null); }
  };
  return { archive, busy: (bot: string, thread: string) => busy === `${bot}:${thread}` };
}

/** Archive of the thread shown, one click: a primary button in the chat's header on the laptop, an icon of the chat's
 * bar on the phone. Disabled while the thread works. */
export function ArchiveThreadButton({ bot, thread, sessions }: { bot: Bot; thread: BotThread; sessions: Session[] }) {
  const phone = usePhone();
  const { archive, busy } = useArchiveThread();
  const running = sessions.some(isRunning);
  const working = busy(bot.id, thread.id);
  const title = running ? "Stop its turn first to archive this thread" : `Archive “${thread.title}”: it stays in ⋯ → Archived threads`;
  const run = () => void archive(bot.id, thread.id, thread.conversation ?? null);
  if (phone) return <IconButton icon="archive" label={running ? title : "Archive this thread"} disabled={running || working} data-tour="bots.archive-thread" onClick={run} />;
  return <button type="button" className="btn btn-sm btn-primary flex-none" disabled={running || working} title={title} data-tour="bots.archive-thread" onClick={run}><Icon name="archive" size={12} />{working ? "Archiving…" : "Archive"}</button>;
}

/** The bot's archived threads, each with Bring back (⋯ → Archived threads). */
export function ArchivedThreads({ bot }: { bot: Bot }) {
  const qc = useQueryClient();
  const threads = useBotThreads(bot.id, true);
  const old = threads.data?.filter((t) => t.archived_at != null) ?? [];
  return <div className="grid grid-cols-1 gap-px">
    {threads.isLoading ? <p className="m-0 px-2 py-1 text-[12px] text-ink-3">Loading…</p>
      : old.length === 0 ? <p className="m-0 px-2 py-1 text-[12px] text-ink-3">No archived thread.</p>
      : old.map((t) => <div key={t.id} className="flex min-h-9 items-center gap-2 px-2 text-[13px] text-ink-2">
        <Icon name="archive" size={12} className="flex-none text-ink-3" /><span className="min-w-0 flex-1 truncate" title={t.title}>{t.title}</span>
        <ActionButton name="bot.thread-archive" params={{ id: bot.id, thread: t.id }} body={{ archived: false }} consequence="Brings the thread and its session back." confirmLabel="Bring back" icon={null} onDone={() => qc.invalidateQueries({ queryKey: ["bot-threads"] })}>Bring back</ActionButton>
      </div>)}
  </div>;
}
