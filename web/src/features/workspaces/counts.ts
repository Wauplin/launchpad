// What waits in each workspace, for the rail (laptop) and the workspace sheet (phone). Other workspaces show these
// counts only, never their content. Same predicates as the sidebar: agents that need you, proposals that wait,
// and Inbox threads for you in the workspace's repositories.

import { useMemo } from "react";
import { forYou, useInbox } from "@/features/inbox/data";
import { useProposals } from "@/features/proposals/count";
import { useProjects, useSessions } from "@/lib/queries";
import { needsYou } from "@/lib/sessionStatus";
import { scopeOf, useWorkspaces } from "@/lib/workspace";

export const workspaceInitial = (name: string) => [...name.trim()][0]?.toUpperCase() ?? "?";

export type WorkspaceCount = { agents: number; proposals: number; inbox: number; total: number };

export function useWorkspaceCounts(): Record<string, WorkspaceCount> {
  const workspaces = useWorkspaces().data;
  const projects = useProjects().data;
  const sessions = useSessions().data;
  const proposals = useProposals().data;
  const threads = useInbox().data?.threads;
  return useMemo(() => {
    const out: Record<string, WorkspaceCount> = {};
    for (const w of workspaces ?? []) {
      const scope = scopeOf(w, projects);
      const agents = (sessions ?? []).filter((s) => needsYou(s) && scope.hasSession(s)).length;
      const waiting = (proposals ?? []).filter((p) => p.state === "proposed" && p.workspace_id === w.id).length;
      const inbox = (threads ?? []).filter((t) => forYou(t) && scope.hasRepo(t.repo)).length;
      out[w.id] = { agents, proposals: waiting, inbox, total: agents + waiting + inbox };
    }
    return out;
  }, [workspaces, projects, sessions, proposals, threads]);
}

/** "2 wait: 1 agent needs you, 1 proposal" for a tooltip or a sheet row. */
export function describeCount(c: WorkspaceCount | undefined): string {
  if (!c?.total) return "Nothing waits";
  const parts = [
    c.agents ? `${c.agents} agent${c.agents === 1 ? " needs" : "s need"} you` : "",
    c.proposals ? `${c.proposals} proposal${c.proposals === 1 ? "" : "s"}` : "",
    c.inbox ? `${c.inbox} Inbox thread${c.inbox === 1 ? "" : "s"} for you` : "",
  ].filter(Boolean);
  return `${c.total} wait${c.total === 1 ? "s" : ""}: ${parts.join(", ")}`;
}
