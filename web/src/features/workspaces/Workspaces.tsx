// Workspaces (ARCHITECTURE.md, 9.1.2): the Settings page that edits them. The top bar switcher is in Switcher.tsx.

import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useState } from "react";
import type { Workspace } from "@/gen/Workspace";
import { Icon } from "@/components/Icon";
import { Card, SettingsPage } from "@/features/settings/SettingsView";
import { toastError } from "@/components/Toast";
import { api } from "@/lib/api";
import { useProjects } from "@/lib/queries";
import { newWindowWord, openWindow, useWorkspaces } from "@/lib/workspace";
import { useWorkspace } from "@/stores/workspace";

function WorkspaceCard({ w }: { w: Workspace }) {
  const qc = useQueryClient();
  const projects = useProjects().data ?? [];
  const { current, switchTo } = useWorkspace();
  const [name, setName] = useState(w.name);
  const [confirm, setConfirm] = useState(false);
  const patch = useMutation({
    mutationFn: (body: { name?: string; project_ids?: string[] }) => api.patch<Workspace>(`/api/workspaces/${w.id}`, body),
    onSettled: () => qc.invalidateQueries({ queryKey: ["workspaces"] }),
    onError: toastError,
  });
  const ids = new Set(w.project_ids);
  const toggle = (id: string) => patch.mutate({ project_ids: ids.has(id) ? w.project_ids.filter((p) => p !== id) : [...w.project_ids, id] });
  // Temporary folders are listed once they belong to the workspace (agents launched from it add them).
  const listed = projects.filter((p) => !p.is_temp || ids.has(p.id));
  return (
    <div className="grid gap-2.5 rounded-lg border border-line p-3.5">
      <div className="phone-workspace-tools flex items-center gap-2">
        <input aria-label="Workspace name" className="field-input h-8 w-56 font-semibold" maxLength={40} value={name} onChange={(e) => setName(e.target.value)} />
        {name.trim() !== w.name && <button className="btn btn-sm" disabled={!name.trim() || patch.isPending} onClick={() => patch.mutate({ name })}>Rename</button>}
        <span className="text-xs text-ink-3">{w.project_ids.length} project{w.project_ids.length === 1 ? "" : "s"}</span>
        <span className="ml-auto flex items-center gap-1.5">
          <button className="btn btn-sm" disabled={current === w.id} onClick={() => switchTo(w.id)}>{current === w.id ? "Shown here" : "Show here"}</button>
          <button className="btn btn-sm" onClick={() => void openWindow(w.id)}><Icon name="window" size={13} />New {newWindowWord}</button>
          <button
            className={`btn btn-sm ${confirm ? "border-[#A40E26]! bg-danger! text-white!" : "btn-ghost"}`}
            title="Delete the workspace (projects and agents are kept)"
            onMouseLeave={() => setConfirm(false)}
            onClick={() => {
              if (!confirm) return setConfirm(true);
              void api.delete(`/api/workspaces/${w.id}`).then(() => qc.invalidateQueries({ queryKey: ["workspaces"] }), toastError);
            }}
          >
            {confirm ? "Click again to delete" : <Icon name="trash" size={13} />}
          </button>
        </span>
      </div>
      <div className="grid max-h-[240px] grid-cols-[repeat(auto-fill,minmax(200px,1fr))] gap-x-3 gap-y-0.5 overflow-y-auto">
        {listed.map((p) => (
          <label key={p.id} className="phone-workspace-project flex min-w-0 items-center gap-2 rounded px-1 py-0.5 text-[12.5px] hover:bg-sunk" title={p.gh_repo ?? p.path}>
            <input type="checkbox" checked={ids.has(p.id)} disabled={patch.isPending} onChange={() => toggle(p.id)} />
            <span className={`truncate ${p.is_temp ? "font-mono text-[11.5px]" : ""}`}>{p.name}</span>
          </label>
        ))}
        {listed.length === 0 && <span className="text-xs text-ink-3">No project yet: add some in <Link to="/projects">Projects</Link>.</span>}
      </div>
    </div>
  );
}

export function WorkspacesView() {
  const qc = useQueryClient();
  const workspaces = useWorkspaces().data ?? [];
  const switchTo = useWorkspace((s) => s.switchTo);
  const [name, setName] = useState("");
  const create = useMutation({
    mutationFn: () => api.post<Workspace>("/api/workspaces", { name, project_ids: [] }),
    onSuccess: (w) => {
      setName("");
      switchTo(w.id);
      void qc.invalidateQueries({ queryKey: ["workspaces"] });
    },
    onError: toastError,
  });
  return (
    <SettingsPage>
      <Card title="Workspaces">
        <p className="m-0 text-xs text-ink-3">A workspace is a set of projects. A window that shows it lists only their agents, scheduled launches, crons and inbox notifications. Each window has its own workspace, picked with the buttons in the top bar; agents launched from a window add their project to its workspace. Changes here are saved at once.</p>
        <div className="flex gap-2">
          <input aria-label="New workspace name" className="field-input h-8 w-64" placeholder="New workspace name" maxLength={40} value={name} onChange={(e) => setName(e.target.value)} />
          <button className="btn" disabled={!name.trim() || create.isPending} onClick={() => create.mutate()}>
            <Icon name="plus" size={13} />
            Create and show here
          </button>
        </div>
        {workspaces.length === 0 && <p className="m-0 text-[12.5px] text-ink-2">No workspace yet. Without one, every window shows all projects.</p>}
        {workspaces.map((w) => <WorkspaceCard key={w.id} w={w} />)}
      </Card>
    </SettingsPage>
  );
}
