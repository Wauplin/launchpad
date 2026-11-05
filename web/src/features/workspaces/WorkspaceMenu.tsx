// The workspace of this window, at the top of the sidebar: its name opens New workspace, Rename, Open in a new
// window, its settings and Delete. The last workspace cannot be deleted (Home refuses it too).

import { useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useState } from "react";
import type { Workspace } from "@/gen/Workspace";
import { Icon } from "@/components/Icon";
import { MenuItem, Popover } from "@/components/Popover";
import { toastError } from "@/components/Toast";
import { ConfirmButton } from "@/components/ui";
import { api } from "@/lib/api";
import { newWindowWord, openWindow, useScope, useWorkspaces } from "@/lib/workspace";
import { useWorkspace } from "@/stores/workspace";

/** Creates a workspace and shows it in this window. */
export function NewWorkspaceForm({ onDone, onCancel }: { onDone: () => void; onCancel: () => void }) {
  const qc = useQueryClient();
  const [name, setName] = useState("");
  const submit = async () => {
    const n = name.trim();
    if (!n) return;
    try {
      const w = await api.post<Workspace>("/api/workspaces", { name: n });
      await qc.invalidateQueries({ queryKey: ["workspaces"] });
      useWorkspace.getState().switchTo(w.id);
      onDone();
    } catch (e) { toastError(e); }
  };
  return (
    <form className="grid gap-2 p-1.5" onSubmit={(e) => { e.preventDefault(); void submit(); }}>
      <label className="grid gap-1 text-[12px] text-ink-2">New workspace name<input autoFocus className="field-input h-[30px]" maxLength={40} value={name} onChange={(e) => setName(e.target.value)} /></label>
      <span className="flex justify-end gap-1.5"><button type="button" className="btn btn-sm btn-ghost" onClick={onCancel}>Cancel</button><button type="submit" className="btn btn-sm btn-primary" disabled={!name.trim()}>Create and show here</button></span>
    </form>
  );
}

export function WorkspaceMenu() {
  const qc = useQueryClient();
  const workspace = useScope().workspace;
  const workspaces = useWorkspaces().data ?? [];
  const [mode, setMode] = useState<"menu" | "new" | "rename">("menu");
  const [name, setName] = useState("");
  const rename = async (close: () => void) => {
    const n = name.trim();
    if (!n) return;
    try {
      await api.patch(`/api/workspaces/${workspace.id}`, { name: n });
      await qc.invalidateQueries({ queryKey: ["workspaces"] });
      close();
      setMode("menu");
    } catch (e) { toastError(e); }
  };
  const last = workspaces.length <= 1;
  return (
    <div className="flex-none border-b border-line-2 px-2 py-1.5">
      <Popover width={280} trigger={({ open, toggle }) => (
        <button className="flex h-[31px] w-full min-w-0 items-center gap-1.5 rounded-[7px] px-[9px] text-left font-semibold text-ink hover:bg-sunk" data-tour="sidebar.workspace-menu" aria-label={`Workspace ${workspace.name}: actions`} title="Workspace of this window" aria-expanded={open} onClick={() => { setMode("menu"); toggle(); }}>
          <span className="min-w-0 flex-1 truncate">{workspace.name}</span>
          <Icon name="chev" size={12} className="flex-none text-ink-3" />
        </button>
      )}>
        {(close) => mode === "new" ? <NewWorkspaceForm onDone={() => { close(); setMode("menu"); }} onCancel={() => setMode("menu")} /> : mode === "rename" ? (
          <form className="grid gap-2 p-1.5" onSubmit={(e) => { e.preventDefault(); void rename(close); }}>
            <label className="grid gap-1 text-[12px] text-ink-2">Rename {workspace.name}<input autoFocus className="field-input h-[30px]" maxLength={40} value={name} onChange={(e) => setName(e.target.value)} /></label>
            <span className="flex justify-end gap-1.5"><button type="button" className="btn btn-sm btn-ghost" onClick={() => setMode("menu")}>Cancel</button><button type="submit" className="btn btn-sm btn-primary" disabled={!name.trim()}>Rename</button></span>
          </form>
        ) : (
          <>
            <MenuItem icon={<Icon name="pencil" size={13} />} label="Rename" onClick={() => { setName(workspace.name); setMode("rename"); }} />
            <MenuItem icon={<Icon name="window" size={13} />} label={`Open in a new ${newWindowWord}`} detail="Two workspaces side by side" onClick={() => { close(); void openWindow(workspace.id); }} />
            <Link to="/settings/workspace" className="flex h-8 items-center gap-2 rounded-md px-2 text-[13px] text-ink no-underline hover:bg-sunk" onClick={close}><Icon name="sliders" size={13} />Workspace settings</Link>
            <MenuItem icon={<Icon name="plus" size={13} />} label="New workspace" detail="Each folder belongs to one workspace" onClick={() => setMode("new")} />
            <div className="border-t border-line-2 px-2 pt-1.5 pb-1">
              {last ? <span className="text-[12px] text-ink-3">The last workspace cannot be deleted.</span>
                : <ConfirmButton icon="trash" consequence="Only an empty workspace can be deleted: move or remove its folders first." confirmLabel={`Delete ${workspace.name}`} onConfirm={async () => { try { await api.delete(`/api/workspaces/${workspace.id}`); await qc.invalidateQueries({ queryKey: ["workspaces"] }); close(); } catch (e) { toastError(e); } }}>Delete workspace</ConfirmButton>}
            </div>
          </>
        )}
      </Popover>
    </div>
  );
}

/** Shown instead of the page while there is no workspace: Home creates one at first start, so only a runner that
 * has not heard from Home yet (or a window whose last workspace was removed elsewhere) gets here. */
export function NoWorkspace() {
  return (
    <div className="grid h-full place-items-center p-6">
      <div className="grid w-[320px] max-w-full gap-2 rounded-lg border border-line p-3">
        <p className="m-0 text-[13px] text-ink-2">Launchpad always shows one workspace, and none exists yet. Create one to start.</p>
        <NewWorkspaceForm onDone={() => undefined} onCancel={() => undefined} />
      </div>
    </div>
  );
}
