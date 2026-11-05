// What the current workspace contains. Every folder belongs to exactly one workspace (`Project.workspace_id`), and a
// window always shows one workspace. Every list that a workspace narrows (sidebar, agents, projects, inbox,
// composer) uses these predicates, so counts and markers agree. Other workspaces show counts only.

import { isDesktopShell, setAppBadge } from "./platform";
import { useQuery } from "@tanstack/react-query";
import { useEffect, useMemo } from "react";
import type { LaunchSpec } from "@/gen/LaunchSpec";
import type { Project } from "@/gen/Project";
import type { Session } from "@/gen/Session";
import type { Workspace } from "@/gen/Workspace";
import { toast } from "@/components/Toast";
import { useWorkspace } from "@/stores/workspace";
import { api } from "./api";
import { inWorkspace, useToday } from "@/features/today/data";
import { useProjects } from "./queries";

export const useWorkspaces = () => useQuery({ queryKey: ["workspaces"], queryFn: () => api.get<Workspace[]>("/api/workspaces"), staleTime: Infinity });

export type Scope = {
  /** The window's workspace. While the workspaces load, a placeholder with an empty id that matches nothing. */
  workspace: Workspace;
  /** The projects of the workspace. */
  projects: Project[];
  hasProject: (id: string | null, cwd?: string) => boolean;
  hasSession: (s: Session) => boolean;
  hasSpec: (spec: LaunchSpec) => boolean;
  /** `owner/repo`, matching a project's origin or upstream. */
  hasRepo: (repo: string) => boolean;
};

const PENDING: Workspace = { id: "", name: "", created_at: 0 };

export function specProject(spec: LaunchSpec): string | null {
  return spec.target.kind === "project" || spec.target.kind === "predraft" ? spec.target.project_id : null;
}

/** The scope of one workspace: its projects and the predicates every narrowed list uses. Also counts other workspaces. */
export function scopeOf(workspace: Workspace | undefined, projects: Project[] | undefined): Scope {
  const w = workspace ?? PENDING;
  const mine = (projects ?? []).filter((p) => !!workspace && p.workspace_id === workspace.id);
  const ids = new Set(mine.map((p) => p.id));
  const repos = new Set(mine.flatMap((p) => [p.gh_repo, p.gh_upstream]).filter((r): r is string => !!r).map((r) => r.toLowerCase()));
  const hasProject = (id: string | null, cwd?: string) => (!!id && ids.has(id)) || (!!cwd && mine.some((p) => cwd === p.path || cwd.startsWith(p.path + "/")));
  const hasSpec = (spec: LaunchSpec) => (!!workspace && spec.workspace_id === workspace.id) || hasProject(specProject(spec));
  return {
    workspace: w,
    projects: mine,
    hasProject,
    hasSession: (s: Session) => hasProject(s.project_id, s.cwd) || hasSpec(s.launch_spec),
    hasSpec,
    hasRepo: (repo: string) => repos.has(repo.toLowerCase()),
  };
}

export function useScope(): Scope {
  const current = useWorkspace((s) => s.current);
  const workspaces = useWorkspaces();
  const projects = useProjects().data;
  return useMemo(() => scopeOf(workspaces.data?.find((w) => w.id === current), projects), [current, workspaces.data, projects]);
}

/** Resolves `?ws=<id or name>`, and picks a workspace when the window has none or its workspace was deleted. Mounted once in the shell. */
export function useWorkspaceSync() {
  const { current, requested, switchTo } = useWorkspace();
  const workspaces = useWorkspaces().data;
  useEffect(() => {
    if (!workspaces?.length) return;
    if (requested !== null) {
      const want = requested.toLowerCase();
      const w = workspaces.find((w) => w.id === requested || w.name.toLowerCase() === want);
      if (!w) toast(`No workspace named ${requested}: showing ${workspaces.find((x) => x.id === current)?.name ?? workspaces[0].name}`);
      switchTo(w?.id ?? (workspaces.some((x) => x.id === current) ? current! : workspaces[0].id));
    } else if (!current || !workspaces.some((w) => w.id === current)) {
      switchTo(workspaces[0].id);
    }
  }, [workspaces, requested, current, switchTo]);
  const name = workspaces?.find((w) => w.id === current)?.name;
  // The same count as Today in the sidebar.
  const today = useToday().data;
  const waiting = (today?.items ?? []).filter((i) => inWorkspace(i, current)).length;
  useEffect(() => { document.title = `${waiting > 0 ? `(${waiting}) ` : ""}${name ? `${name} · ` : ""}Launchpad`; }, [name, waiting]);
  // The installed app's badge is per app: every workspace counts.
  const allWaiting = today?.items.length ?? 0;
  useEffect(() => { if (today) setAppBadge(allWaiting); }, [today, allWaiting]);
  /** No workspace yet (a runner before Home answered, or every workspace deleted elsewhere): the shell asks for one. */
  return { empty: workspaces?.length === 0 };
}

/** Opens another window on a workspace: a desktop window in the desktop shell, a browser tab otherwise. */
export async function openWindow(workspaceId: string) {
  const url = `/?ws=${encodeURIComponent(workspaceId)}`;
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

/** Moves a folder to another workspace (or assigns an Unassigned one); every window refreshes on the `projects` bus topic. */
export async function moveFolder(project: Project, workspace: Workspace) {
  await api.patch<Project>(`/api/projects/${project.id}`, { workspace_id: workspace.id });
}

