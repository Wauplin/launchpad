// Bot channels and DMs (`launchpad_channels::channel`, Home's records): named conversations of a workspace's bots, and
// DMs of two bots. The owner is a member of every one. A channel shows a plain unread dot for new messages and an
// amber count only when a bot asks the owner or mentions the owner there: its Today row (`useChannelAsks`).
// The pure helpers are in channelRows.ts (tested), re-exported here.

import { useQuery } from "@tanstack/react-query";
import type { Channel } from "@/gen/Channel";
import { api } from "./api";

export const useChannels = () => useQuery({ queryKey: ["channels"], queryFn: () => api.get<Channel[]>("/api/channels"), staleTime: 30_000, refetchInterval: 60_000, retry: false });

/** A channel's name for lists ("#release-train", "Maintainer ↔ Release"), by its ID. */
export function useChannelName(): (id: string) => string {
  const channels = useChannels().data ?? [];
  return (id) => channels.find((c) => c.id === id)?.title ?? "a removed channel";
}

export * from "./channelRows";
