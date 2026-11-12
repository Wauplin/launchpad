// Today's data: the decision list Home computes (GET /api/today, crates/core/src/today.rs), shared by the page, the
// sidebar count, the workspace menu and phone sheet badges, the phone tab and the window title. Kept apart from the page so
// those do not load it. The `today` bus topic refreshes it.

import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import type { Today } from "@/gen/Today";
import type { TodayFilter } from "@/gen/TodayFilter";
import type { TodayItem } from "@/gen/TodayItem";
import { api } from "@/lib/api";
import { useWorkspace } from "@/stores/workspace";

export type { TodayItem, TodayFilter };

// Runners do not receive Home's bus events: a slow poll keeps their windows current.
export const useToday = () => useQuery({ queryKey: ["today"], queryFn: () => api.get<Today>("/api/today"), staleTime: 15_000, refetchInterval: 60_000 });

/** An item with an empty workspace list (shared: a runner to update, a tool or preset proposal) shows in every workspace. */
export const inWorkspace = (i: TodayItem, workspace: string | null) => !i.workspaces.length || (!!workspace && i.workspaces.includes(workspace));

/** The chips of the page: each filter has its own address (`/?filter=send`), for Telegram links and Ask cards. */
export const FILTERS: [TodayFilter, string, string][] = [
  ["answer", "Answer", "Questions agents asked you, and bots that ask you in a channel"],
  ["review", "Review", "Finished, failed or interrupted agents you have not seen, and Local PRs to publish"],
  ["approve", "Approve", "Proposal cards agents and bots wrote"],
  ["send", "Send", "Messages agents drafted for other people"],
  ["github", "GitHub", "Suggestions to post and pull requests that request your review"],
  ["fix", "Fix", "Routines whose last run failed and runners to update"],
];
export type TodayView = "all" | TodayFilter | "done";
export const isView = (v: unknown): v is TodayView => v === "all" || v === "done" || FILTERS.some(([f]) => f === v);

/** The window's workspace: what waits and what was handled. */
export function useTodayHere(): { items: TodayItem[]; done: TodayItem[]; loading: boolean; error: unknown } {
  const current = useWorkspace((s) => s.current);
  const q = useToday();
  return useMemo(() => ({
    items: (q.data?.items ?? []).filter((i) => inWorkspace(i, current)),
    done: (q.data?.done ?? []).filter((i) => inWorkspace(i, current)),
    loading: q.isPending,
    error: q.error,
  }), [q.data, q.isPending, q.error, current]);
}

/** The one count of the sidebar, the phone tab and the window title. */
export const useTodayCount = () => useTodayHere().items.length;

/** The channels of the window's workspace where a bot asks you (Today's `channel_ask` items, one per channel): channel
 * ID → how many messages wait for you. The channel rows' amber counts and the Chats tab read only this. */
export function useChannelAsks(): Map<string, number> {
  const { items } = useTodayHere();
  return useMemo(() => new Map(items.filter((i) => i.kind === "channel_ask").map((i) => [i.record, i.count ?? 1])), [items]);
}
