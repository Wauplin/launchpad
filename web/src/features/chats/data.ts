// The Chats data of the window's workspace: every conversation of its bots (each bot's threads, archived ones too, and
// the session store) and its channels, as `lib/chatList.ts` rows, with your pins (`chat.pin`, on Home, so every
// device shows the same shortcuts).

import { keepPreviousData, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo } from "react";
import { toastError } from "@/components/Toast";
import type { BotThread } from "@/gen/BotThread";
import { runAction } from "@/lib/actions";
import { api } from "@/lib/api";
import { useBots } from "@/lib/bots";
import { useChannels } from "@/lib/channels";
import { buildChats, byTitle, type Chat, type ChatInputs } from "@/lib/chatList";
import { useChatsStore } from "@/stores/chats";
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

/** The Chats list's threads (`GET /api/chat-threads`): every open thread of the workspace's bots and the `archived`
 * newest archived conversations, with how many archived conversations there are. One request for the workspace,
 * refreshed by the `bots`/`threads` bus event; Show more raises `archived`. */
export type ChatThreads = { threads: BotThread[]; archived_total: number };
export function useChatThreads() {
  const ws = useScope().workspace.id;
  const archived = useChatsStore((s) => s.archivedLimit);
  return useQuery({ queryKey: ["chat-threads", ws, archived], queryFn: () => api.get<ChatThreads>(`/api/chat-threads?workspace=${encodeURIComponent(ws ?? "")}&archived=${archived}`), enabled: !!ws, staleTime: 15_000, retry: false, placeholderData: keepPreviousData });
}

// Several parts of a window read the chats at once (the sidebar, the list, a chat's header): the last result is
// shared while its inputs are the same records.
let last: { inputs: unknown[]; chats: Chat[] } | null = null;
function sharedChats(inputs: ChatInputs, keys: unknown[]): Chat[] {
  if (last && last.inputs.length === keys.length && last.inputs.every((v, i) => v === keys[i])) return last.chats;
  const chats = buildChats(inputs);
  last = { inputs: keys, chats };
  return chats;
}

/** Every chat of the workspace (newest activity first), whether the lists are still loading, the pins, and how many
 * archived conversations are not loaded yet. */
export function useChats(): { chats: Chat[]; loading: boolean; error: Error | null; pinned: Set<string>; archivedMore: number } {
  const scope = useScope();
  const ws = scope.workspace.id;
  const botsQuery = useBots();
  const sessions = useSessions().data;
  const channels = useChannels().data;
  const asks = useChannelAsks();
  const pins = useChatPins().data;
  const threads = useChatThreads().data;
  const chats = useMemo(() => {
    const mine = (botsQuery.data ?? []).filter((b) => b.workspace_id === ws);
    // Loaded: every bot has its list (possibly empty); while loading, the threads its sessions name stand in.
    const byBot = new Map<string, BotThread[] | undefined>(mine.map((b) => [b.id, threads ? threads.threads.filter((t) => t.bot_id === b.id) : undefined]));
    return sharedChats({ bots: mine, sessions: sessions ?? [], threads: byBot, channels, workspaceId: ws, asks }, [botsQuery.data, sessions, channels, ws, asks, threads]);
  }, [botsQuery.data, sessions, channels, ws, asks, threads]);
  const pinned = useMemo(() => new Set((pins ?? []).map((p) => p.target)), [pins]);
  const loadedArchived = useMemo(() => new Set(chats.filter((c) => c.kind === "thread" && c.archived).map((c) => c.id)).size, [chats]);
  return { chats, loading: botsQuery.isLoading, error: botsQuery.error, pinned, archivedMore: Math.max(0, (threads?.archived_total ?? 0) - loadedArchived) };
}

/** The pinned chats on the laptop's sidebar, in alphabetical order of their titles: open chats only (an archived chat
 * keeps its pin and shows again once brought back; the phone's list does the same). Pins whose chat is gone are left out. */
export function usePinnedChats(): Chat[] {
  const { chats } = useChats();
  const pins = useChatPins().data;
  return useMemo(() => {
    const byPin = new Map(chats.map((c) => [c.pin, c]));
    return byTitle((pins ?? []).map((p) => byPin.get(p.target)).filter((c): c is Chat => !!c && !c.archived));
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
