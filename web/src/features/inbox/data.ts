// Inbox data shared by the page, the sidebar count and the phone tab badge, kept apart
// from the page so those do not load it.

import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import { create } from "zustand";
import type { InboxSnapshot } from "@/gen/InboxSnapshot";
import { api } from "@/lib/api";
import { useScope } from "@/lib/workspace";

export const useInbox = () => useQuery({ queryKey: ["inbox"], queryFn: () => api.get<InboxSnapshot>("/api/inbox"), staleTime: Infinity });
/** `you`: unread review requests, mentions and assignments, the number the sidebar shows. */
export type Why = "you" | "review" | "mention" | "assign" | "other" | "all";
export const useView = create<{ selected: string | null; repo: string | null; why: Why; unread: boolean; scroll: number; }>(() => ({ selected: null, repo: null, why: "all", unread: false, scroll: 0 }));
export const whyOf = (reason: string): Exclude<Why, "all" | "you"> => reason === "review_requested" ? "review" : reason === "mention" || reason === "team_mention" ? "mention" : reason === "assign" ? "assign" : "other";
export const forYou = (t: { unread: boolean; reason: string }) => t.unread && whyOf(t.reason) !== "other";
export const whys: [Why, string, string][] = [["you", "For you", "Unread review requests, mentions and assignments: the number next to Inbox in the sidebar"], ["review", "Review requested", "Someone requested your review"], ["mention", "Mentioned", "You or your team were mentioned"], ["assign", "Assigned", "You are assigned"], ["other", "Subscribed", "Subscribed, authored, commented, CI activity and state changes"], ["all", "All", "Every notification"]];
/** "For you": unread notifications that need you: review requests, mentions and assignments. The sidebar shows this count. */
export function useInboxNeeds() {
  const { threads } = useInboxThreads();
  return useMemo(() => threads.filter(forYou).length, [threads]);
}
/** Threads of the workspace's repositories. Repositories without a folder belong to no workspace: Settings →
 * GitHub lists them, and no Inbox shows them. The sidebar count uses the same list. */
export function useInboxThreads() {
  const { data } = useInbox();
  const scope = useScope();
  const threads = useMemo(() => (data?.threads ?? []).filter((t) => scope.hasRepo(t.repo)), [data, scope]);
  return { threads, scope };
}
