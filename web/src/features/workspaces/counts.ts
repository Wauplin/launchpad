// What waits in each workspace, for the rail (laptop) and the workspace sheet (phone): Today's count of each
// workspace, from the decision list Home computes. Other workspaces show these counts only, never their content.

import { useMemo } from "react";
import { inWorkspace, useToday } from "@/features/today/data";
import { useWorkspaces } from "@/lib/workspace";

export const workspaceInitial = (name: string) => [...name.trim()][0]?.toUpperCase() ?? "?";

export type WorkspaceCount = { total: number };

export function useWorkspaceCounts(): Record<string, WorkspaceCount> {
  const workspaces = useWorkspaces().data;
  const items = useToday().data?.items;
  return useMemo(() => {
    const out: Record<string, WorkspaceCount> = {};
    for (const w of workspaces ?? []) out[w.id] = { total: (items ?? []).filter((i) => inWorkspace(i, w.id)).length };
    return out;
  }, [workspaces, items]);
}

/** "3 decisions wait in Today" for a tooltip or a sheet row. */
export function describeCount(c: WorkspaceCount | undefined): string {
  if (!c?.total) return "Nothing waits";
  return `${c.total} decision${c.total === 1 ? " waits" : "s wait"} in Today`;
}
