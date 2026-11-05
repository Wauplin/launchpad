// Runners as Home sees them. Only Home has runners; elsewhere the query stays off.

import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import type { RunnerStatus } from "@/gen/RunnerStatus";
import { api } from "./api";
import { useStatus } from "./queries";

export function useRunners() {
  const home = useStatus().data?.role === "home";
  return useQuery({ queryKey: ["runners"], queryFn: () => api.get<RunnerStatus[]>("/api/runners"), enabled: home, staleTime: Infinity });
}

/** Home: names of paired runners that are offline. Their task worktrees are not listed until they reconnect. */
export function useOfflineRunners(): string[] {
  const list = useRunners().data;
  return useMemo(() => (list ?? []).filter((r) => r.runner.state === "approved" && !r.connected).map((r) => r.runner.name), [list]);
}

/** A short note for lists of task worktrees on Home, or null. */
export function offlineWorktreesNote(names: string[]): string | null {
  if (names.length === 0) return null;
  return `${names.join(", ")} ${names.length === 1 ? "is" : "are"} offline: ${names.length === 1 ? "its" : "their"} agent branches show when ${names.length === 1 ? "it reconnects" : "they reconnect"}.`;
}

/** Home: connected runners with another build than Home's. They take no new sessions until updated. */
export function useOutdatedRunners(): RunnerStatus[] {
  const list = useRunners().data;
  return useMemo(() => (list ?? []).filter((r) => r.runner.state === "approved" && r.connected && r.compat === "outdated"), [list]);
}

/** "A needs an update" / "A and B need an update". */
export function outdatedLabel(names: string[]): string {
  const list = names.length <= 2 ? names.join(" and ") : `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;
  return `${list} ${names.length === 1 ? "needs" : "need"} an update`;
}
