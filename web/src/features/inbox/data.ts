// Inbox data shared by the page, the sidebar count, the phone tab badge and the Review tab, kept apart
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
/** `allRepos`: show every repository instead of the workspace's ones. */
export const useView = create<{ selected: string | null; repo: string | null; why: Why; unread: boolean; allRepos: boolean; scroll: number; }>(() => ({ selected: null, repo: null, why: "all", unread: false, allRepos: false, scroll: 0 }));
export const whyOf = (reason: string): Exclude<Why, "all" | "you"> => reason === "review_requested" ? "review" : reason === "mention" || reason === "team_mention" ? "mention" : reason === "assign" ? "assign" : "other";
export const forYou = (t: { unread: boolean; reason: string }) => t.unread && whyOf(t.reason) !== "other";
export const whys: [Why, string, string][] = [["you", "For you", "Unread review requests, mentions and assignments: the number next to Inbox in the sidebar"], ["review", "Review requested", "Someone requested your review"], ["mention", "Mentioned", "You or your team were mentioned"], ["assign", "Assigned", "You are assigned"], ["other", "Subscribed", "Subscribed, authored, commented, CI activity and state changes"], ["all", "All", "Every notification"]];
/** "For you": unread notifications that need you: review requests, mentions and assignments. The sidebar shows this count. */
export function useInboxNeeds() {
  const { threads } = useInboxThreads();
  return useMemo(() => threads.filter(forYou).length, [threads]);
}
/** Threads of the workspace's repositories, unless All repos is on. The sidebar count uses the same list. */
export function useInboxThreads() {
  const { data } = useInbox();
  const scope = useScope();
  const allRepos = useView((s) => s.allRepos);
  const threads = useMemo(() => data?.threads ?? [], [data]);
  const scoped = useMemo(() => (allRepos || !scope.workspace ? threads : threads.filter((t) => scope.hasRepo(t.repo))), [threads, allRepos, scope]);
  return { threads: scoped, total: threads.length, scope, allRepos };
}
