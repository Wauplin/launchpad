// The conversation of the thread a bot page shows (`BotThread.conversation`): its bots, each with its thread and
// sessions, from the same queries as Active threads (each bot's thread list, refreshed by the `bots`/`threads` bus
// event, and the session store). A thread no other bot joined is a conversation of one bot.

import { useQueries } from "@tanstack/react-query";
import { useMemo } from "react";
import type { Bot } from "@/gen/Bot";
import type { BotThread } from "@/gen/BotThread";
import type { Session } from "@/gen/Session";
import { api } from "@/lib/api";
import { botOf, threadOf, useBots } from "@/lib/bots";
import { mainBotOf, type ConversationBot } from "@/lib/conversations";
import { useSessions } from "@/lib/queries";

/** A bot of the conversation: its thread and that thread's sessions, oldest first. */
export type Participant = { bot: Bot; thread: BotThread; sessions: Session[] };

export type ConversationInfo = {
  id: string;
  mainId: string;
  /** Its bots with an open thread in it, the main bot first. */
  participants: Participant[];
  /** The workspace's bots, for @-mentions. */
  bots: ConversationBot[];
};

/** The conversation of `threadId` of `bot` (null for a new thread, or while its thread list loads). */
export function useConversation(bot: Bot, threadId: string | null): ConversationInfo | null {
  const all = useBots().data;
  const sessions = useSessions().data;
  const mine = useMemo(() => (all ?? []).filter((b) => b.workspace_id === bot.workspace_id), [all, bot.workspace_id]);
  const lists = useQueries({ queries: mine.map((b) => ({ queryKey: ["bot-threads", b.id, false], queryFn: () => api.get<BotThread[]>(`/api/bots/${b.id}/threads`), staleTime: 15_000, retry: false })) });
  const data = lists.map((l) => l.data);
  return useMemo(() => {
    if (!threadId) return null;
    const own = data[mine.findIndex((b) => b.id === bot.id)]?.find((t) => t.id === threadId);
    if (!own) return null;
    const id = own.conversation;
    const mainId = mainBotOf(id);
    const participants: Participant[] = [];
    mine.forEach((b, i) => {
      const t = b.id === bot.id ? own : data[i]?.find((x) => x.conversation === id && x.archived_at == null);
      if (!t) return;
      const list = (sessions ?? []).filter((s) => botOf(s) === b.id && threadOf(s) === t.id).sort((x, y) => x.started_at - y.started_at);
      participants.push({ bot: b, thread: t, sessions: list });
    });
    participants.sort((a, b) => Number(b.bot.id === mainId) - Number(a.bot.id === mainId) || a.bot.settings.name.localeCompare(b.bot.settings.name));
    return { id, mainId, participants, bots: mine.map((b) => ({ id: b.id, slug: b.slug, name: b.settings.name })) };
  }, [threadId, bot.id, mine, sessions, ...data]); // eslint-disable-line react-hooks/exhaustive-deps
}
