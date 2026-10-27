// What the current workspace contains. Every list that a workspace narrows (sidebar, agents, projects,
// inbox, composer) uses these predicates, so counts and markers agree.

import { isDesktopShell } from "./platform";
import { useQuery } from "@tanstack/react-query";
import { useEffect, useMemo } from "react";
import type { LaunchSpec } from "@/gen/LaunchSpec";
import type { Project } from "@/gen/Project";
import type { Session } from "@/gen/Session";
import type { Workspace } from "@/gen/Workspace";
import { toast } from "@/components/Toast";
import { useWorkspace } from "@/stores/workspace";
import { api } from "./api";
import { setAppBadge } from "./push";
import { useProjects, useSessions } from "./queries";
import { needsYou } from "./sessionStatus";

export const useWorkspaces = () => useQuery({ queryKey: ["workspaces"], queryFn: () => api.get<Workspace[]>("/api/workspaces"), staleTime: Infinity });

export type Scope = {
  /** null: all workspaces. */
  workspace: Workspace | null;
  /** The projects of the workspace (every project when there is none). */
  projects: Project[];
  hasProject: (id: string | null, cwd?: string) => boolean;
  hasSession: (s: Session) => boolean;
  hasSpec: (spec: LaunchSpec) => boolean;
  /** `owner/repo`, matching a project's origin or upstream. */
  hasRepo: (repo: string) => boolean;
};

const yes = () => true;

export function specProject(spec: LaunchSpec): string | null {
  return spec.target.kind === "project" || spec.target.kind === "predraft" ? spec.target.project_id : null;
}

export function useScope(): Scope {
  const current = useWorkspace((s) => s.current);
  const workspaces = useWorkspaces();
  const projects = useProjects().data;
  return useMemo(() => {
    const workspace = workspaces.data?.find((w) => w.id === current) ?? null;
    // While the workspaces load, show nothing rather than every project.
    if (!workspace && !(current && !workspaces.data)) return { workspace, projects: projects ?? [], hasProject: yes, hasSession: yes, hasSpec: yes, hasRepo: yes };
    const ids = new Set(workspace?.project_ids ?? []);
    const mine = (projects ?? []).filter((p) => ids.has(p.id));
    const repos = new Set(mine.flatMap((p) => [p.gh_repo, p.gh_upstream]).filter((r): r is string => !!r).map((r) => r.toLowerCase()));
    const hasProject = (id: string | null, cwd?: string) => (!!id && ids.has(id)) || (!!cwd && mine.some((p) => cwd === p.path || cwd.startsWith(p.path + "/")));
    const hasSpec = (spec: LaunchSpec) => (!!workspace && spec.workspace_id === workspace.id) || hasProject(specProject(spec));
    return {
      workspace,
      projects: mine,
      hasProject,
      hasSession: (s: Session) => hasProject(s.project_id, s.cwd) || hasSpec(s.launch_spec),
      hasSpec,
      hasRepo: (repo: string) => repos.has(repo.toLowerCase()),
    };
  }, [current, workspaces.data, projects]);
}

/** Resolves `?ws=<id or name>` and forgets a workspace deleted from another window. Mounted once in the shell. */
export function useWorkspaceSync() {
  const { current, requested, switchTo } = useWorkspace();
  const workspaces = useWorkspaces().data;
  useEffect(() => {
    if (!workspaces) return;
    if (requested !== null) {
      const want = requested.toLowerCase();
      const w = workspaces.find((w) => w.id === requested || w.name.toLowerCase() === want);
      if (!w && want !== "" && want !== "all") toast(`No workspace named ${requested}: showing all workspaces`);
      switchTo(w?.id ?? null);
    } else if (current && !workspaces.some((w) => w.id === current)) {
      switchTo(null);
    }
  }, [workspaces, requested, current, switchTo]);
  const name = workspaces?.find((w) => w.id === current)?.name;
  // Same predicate and scope as the sidebar's "N waiting" count.
  const scope = useScope();
  const sessions = useSessions().data;
  const waiting = (sessions ?? []).filter((s) => scope.hasSession(s) && needsYou(s)).length;
  useEffect(() => { document.title = `${waiting > 0 ? `(${waiting}) ` : ""}${name ? `${name} · ` : ""}Launchpad`; }, [name, waiting]);
  // The installed app's badge is per app, like push notifications: every workspace counts.
  const allWaiting = (sessions ?? []).filter(needsYou).length;
  useEffect(() => { if (sessions) setAppBadge(allWaiting); }, [sessions, allWaiting]);
}

/** Opens another window on a workspace (null: all workspaces): a desktop window in the desktop shell, a browser tab otherwise. */
export async function openWindow(workspaceId: string | null) {
  const url = `/?ws=${encodeURIComponent(workspaceId ?? "all")}`;
  if (!isDesktopShell) {
    window.open(url, "_blank");
    return;
  }
  try {
    await api.post("/api/window", { workspace_id: workspaceId });
  } catch {
    window.open(url, "_blank");
  }
}

/** "window" in the desktop shell, "tab" in a browser. */
export const newWindowWord = isDesktopShell ? "window" : "tab";

/** Membership changes of a workspace, saved at once; every window refreshes on the `workspaces` bus topic. */
export async function setWorkspaceProjects(workspace: Workspace, projectIds: string[]) {
  await api.patch<Workspace>(`/api/workspaces/${workspace.id}`, { project_ids: projectIds });
}

/** After a launch that silently added its folder to the window's workspace: "Added X to <workspace> · Undo". */
export function announceImplicitAdd(workspace: Workspace | null, projectId: string | null, name: string) {
  if (!workspace || !projectId || workspace.project_ids.includes(projectId)) return;
  toast(`Added ${name} to ${workspace.name}`, {
    label: "Undo",
    run: () => void api.get<Workspace[]>("/api/workspaces").then((all) => {
      const w = all.find((x) => x.id === workspace.id);
      if (w) return setWorkspaceProjects(w, w.project_ids.filter((id) => id !== projectId));
    }).catch(() => toast(`Could not remove ${name} from ${workspace.name}`)),
  });
}
