// The Chats data of the window's workspace: every conversation of its bots (each bot's threads, archived ones too, and
// the session store) and its channels, as `lib/chatList.ts` rows, with your pins (`chat.pin`, on Home, so every
// device shows the same shortcuts).

import { useQueries, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo } from "react";
import { toastError } from "@/components/Toast";
import type { BotThread } from "@/gen/BotThread";
import { runAction } from "@/lib/actions";
import { api } from "@/lib/api";
import { useBots } from "@/lib/bots";
import { useChannels } from "@/lib/channels";
import { buildChats, type Chat } from "@/lib/chatList";
import { useSessions } from "@/lib/queries";
import { useScope } from "@/lib/workspace";
import { useChannelAsks, useTodayHere } from "@/features/today/data";

export type ChatPin = { target: string; workspace_id: string; at: number };

/** Your pinned chats in this workspace, oldest pin first. */
export function useChatPins() {
  const ws = useScope().workspace.id;
  return useQuery({ queryKey: ["chat-pins", ws], queryFn: () => api.get<ChatPin[]>(`/api/chat-pins?workspace=${encodeURIComponent(ws ?? "")}`), enabled: !!ws, staleTime: 60_000, retry: false });
}

/** Pin or unpin a chat (a shortcut in the laptop sidebar, first in the phone's list). */
export function usePinChat() {
  const qc = useQueryClient();
  const ws = useScope().workspace.id;
  return async (chat: Pick<Chat, "pin">, pinned: boolean) => {
    if (!ws) return;
    // Shown at once; Home's answer and the bus event confirm it.
    qc.setQueryData<ChatPin[]>(["chat-pins", ws], (old) => pinned ? [...(old ?? []).filter((p) => p.target !== chat.pin), { target: chat.pin, workspace_id: ws, at: Date.now() }] : (old ?? []).filter((p) => p.target !== chat.pin));
    try { await runAction("chat.pin", {}, { workspace_id: ws, target: chat.pin, pinned }); }
    catch (e) { toastError(e); }
    finally { void qc.invalidateQueries({ queryKey: ["chat-pins", ws] }); }
  };
}

/** Every chat of the workspace (newest activity first) and whether the lists are still loading. */
export function useChats(): { chats: Chat[]; loading: boolean; error: Error | null; pinned: Set<string> } {
  const scope = useScope();
  const ws = scope.workspace.id;
  const botsQuery = useBots();
  const sessions = useSessions().data;
  const channels = useChannels().data;
  const asks = useChannelAsks();
  const pins = useChatPins().data;
  const mine = useMemo(() => (botsQuery.data ?? []).filter((b) => b.workspace_id === ws), [botsQuery.data, ws]);
  // Archived threads too: the list's Archived section. Refreshed by the `bots`/`threads` bus event.
  const lists = useQueries({ queries: mine.map((b) => ({ queryKey: ["bot-threads", b.id, true], queryFn: () => api.get<BotThread[]>(`/api/bots/${b.id}/threads?archived=true`), staleTime: 15_000, retry: false })) });
  const data = lists.map((l) => l.data);
  const chats = useMemo(() => buildChats({ bots: mine, sessions: sessions ?? [], threads: new Map(mine.map((b, i) => [b.id, data[i]])), channels, workspaceId: ws, asks }),
    [mine, sessions, channels, ws, asks, ...data]); // eslint-disable-line react-hooks/exhaustive-deps
  const pinned = useMemo(() => new Set((pins ?? []).map((p) => p.target)), [pins]);
  return { chats, loading: botsQuery.isLoading, error: botsQuery.error, pinned };
}

/** The pinned chats in the order you pinned them (pins whose chat is gone are left out). */
export function usePinnedChats(): Chat[] {
  const { chats } = useChats();
  const pins = useChatPins().data;
  return useMemo(() => {
    const byPin = new Map(chats.map((c) => [c.pin, c]));
    return (pins ?? []).map((p) => byPin.get(p.target)).filter((c): c is Chat => !!c);
  }, [chats, pins]);
}

/** How many chats need you, read from Today like every attention count: the cards of bot sessions (a question, an
 * unseen finished or failed turn; one card per conversation) and the channels where a bot asks you. */
export function useChatsNeedCount(): number {
  const { items } = useTodayHere();
  const sessions = useSessions().data;
  return useMemo(() => {
    const bots = new Set((sessions ?? []).filter((s) => s.launch_spec.bot).map((s) => s.id));
    return items.filter((i) => i.kind === "channel_ask" || ((i.kind === "question" || i.kind === "finished" || i.kind === "failed") && bots.has(i.record))).length;
  }, [items, sessions]);
}
