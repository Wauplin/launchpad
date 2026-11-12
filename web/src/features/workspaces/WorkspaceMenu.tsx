// The workspace of this window, at the top of the sidebar: its name opens the list of workspaces (a click shows one in
// this window; the others show what waits in them as a count, never their content), then Rename, Open in a new tab,
// its settings, New workspace and Delete. The last workspace cannot be deleted (Home refuses it too). The trigger
// carries the total count of the other workspaces. With the sidebar narrowed to icons, the trigger is the workspace's
// initial.

import { useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useState } from "react";
import type { Workspace } from "@/gen/Workspace";
import { Icon } from "@/components/Icon";
import { MenuItem, Popover } from "@/components/Popover";
import { toastError } from "@/components/Toast";
import { ConfirmButton, Count } from "@/components/ui";
import { api } from "@/lib/api";
import { openInNewTab, useScope, useWorkspaces } from "@/lib/workspace";
import { useWorkspace } from "@/stores/workspace";
import { describeCount, useWorkspaceCounts, workspaceInitial } from "./counts";

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
      <label className="grid gap-1 text-[12px] text-ink-2">New workspace name<input autoFocus className="field-input" maxLength={40} value={name} onChange={(e) => setName(e.target.value)} /></label>
      <span className="flex justify-end gap-1.5"><button type="button" className="btn btn-sm btn-ghost" onClick={onCancel}>Cancel</button><button type="submit" className="btn btn-sm btn-primary" disabled={!name.trim()}>Create and show here</button></span>
    </form>
  );
}

export function WorkspaceMenu({ narrow = false }: { narrow?: boolean }) {
  const qc = useQueryClient();
  const workspace = useScope().workspace;
  const workspaces = useWorkspaces().data ?? [];
  const counts = useWorkspaceCounts();
  const elsewhere = workspaces.reduce((n, w) => n + (w.id === workspace.id ? 0 : counts[w.id]?.total ?? 0), 0);
  const elsewhereTitle = `${elsewhere} decision${elsewhere === 1 ? " waits" : "s wait"} in other workspaces`;
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
    <div className={`flex-none border-b border-line-2 py-1.5 ${narrow ? "px-1.5" : "px-2"}`} data-tour="sidebar.workspaces">
      <Popover width={280} trigger={({ open, toggle }) => narrow ? (
        <button className="relative mx-auto grid size-[31px] place-items-center rounded-control bg-teal text-[13px] font-semibold text-white hover:opacity-90" data-tour="sidebar.workspace-menu" aria-label={`Workspace ${workspace.name}: switch and actions`} title={`${workspace.name} · workspace of this window${elsewhere ? `. ${elsewhereTitle}` : ""}`} aria-expanded={open} onClick={() => { setMode("menu"); toggle(); }}>
          {workspaceInitial(workspace.name)}
          {elsewhere > 0 && <span className="absolute -top-1.5 -right-1.5 min-w-[17px] rounded-full border-2 border-chrome bg-attn px-[3px] text-center text-[11px] leading-[13px] font-bold text-white tabular-nums">{elsewhere}</span>}
        </button>
      ) : (
        <button className="flex h-[31px] w-full min-w-0 items-center gap-1.5 rounded-control px-[9px] text-left font-semibold text-ink hover:bg-sunk" data-tour="sidebar.workspace-menu" aria-label={`Workspace ${workspace.name}: switch and actions`} title="Workspace of this window: switch, rename, create" aria-expanded={open} onClick={() => { setMode("menu"); toggle(); }}>
          <span className="min-w-0 flex-1 truncate">{workspace.name}</span>
          <Count n={elsewhere} needs title={elsewhereTitle} />
          <Icon name="chev" size={12} className="flex-none text-ink-3" />
        </button>
      )}>
        {(close) => mode === "new" ? <NewWorkspaceForm onDone={() => { close(); setMode("menu"); }} onCancel={() => setMode("menu")} /> : mode === "rename" ? (
          <form className="grid gap-2 p-1.5" onSubmit={(e) => { e.preventDefault(); void rename(close); }}>
            <label className="grid gap-1 text-[12px] text-ink-2">Rename {workspace.name}<input autoFocus className="field-input" maxLength={40} value={name} onChange={(e) => setName(e.target.value)} /></label>
            <span className="flex justify-end gap-1.5"><button type="button" className="btn btn-sm btn-ghost" onClick={() => setMode("menu")}>Cancel</button><button type="submit" className="btn btn-sm btn-primary" disabled={!name.trim()}>Rename</button></span>
          </form>
        ) : (
          <>
            {workspaces.length > 0 && <div className="grid gap-px border-b border-line-2 pb-1 mb-1" role="group" aria-label="Workspaces">
              {workspaces.map((w) => {
                const here = w.id === workspace.id;
                const n = here ? 0 : counts[w.id]?.total ?? 0;
                return (
                  <button key={w.id} type="button" aria-label={w.name} aria-pressed={here} title={here ? `${w.name} · shown in this window` : `${w.name} · ${describeCount(counts[w.id])}. Click to show it here.`} className={`flex h-8 min-w-0 items-center gap-2 rounded-control px-2 text-left text-[13px] text-ink hover:bg-select ${here ? "font-semibold" : ""}`} onClick={() => { close(); if (!here) useWorkspace.getState().switchTo(w.id); }}>
                    <span className={`grid size-[18px] flex-none place-items-center rounded text-[11px] font-semibold ${here ? "bg-teal text-white" : "bg-sunk text-ink-2"}`} aria-hidden>{workspaceInitial(w.name)}</span>
                    <span className="min-w-0 flex-1 truncate">{w.name}</span>
                    {here ? <Icon name="check" size={12} className="flex-none text-teal-ink" /> : <Count n={n} needs />}
                  </button>
                );
              })}
            </div>}
            <MenuItem icon={<Icon name="pencil" size={12} />} label={`Rename ${workspace.name}`} onClick={() => { setName(workspace.name); setMode("rename"); }} />
            <MenuItem icon={<Icon name="window" size={12} />} label="Open in a new tab" detail="Two workspaces side by side" onClick={() => { close(); openInNewTab(workspace.id); }} />
            <Link to="/settings/workspace" className="flex h-8 items-center gap-2 rounded-control px-2 text-[13px] text-ink no-underline hover:bg-sunk" onClick={close}><Icon name="sliders" size={12} />Workspace settings</Link>
            <MenuItem icon={<Icon name="plus" size={12} />} label="New workspace" detail="Each folder belongs to one workspace" onClick={() => setMode("new")} />
            <div className="border-t border-line-2 px-2 pt-1.5 pb-1">
              {last ? <span className="text-[12px] text-ink-3">The last workspace cannot be deleted.</span>
                : <ConfirmButton icon="trash" consequence="Only an empty workspace can be deleted: move or remove its folders first. Its bots are deleted with it." confirmLabel={`Delete ${workspace.name}`} onConfirm={async () => { try { await api.delete(`/api/workspaces/${workspace.id}`); await qc.invalidateQueries({ queryKey: ["workspaces"] }); close(); } catch (e) { toastError(e); } }}>Delete workspace</ConfirmButton>}
            </div>
          </>
        )}
      </Popover>
    </div>
  );
}

/** Shown instead of the page while there is no workspace: there is no default one, so a new Home (or a window whose
 * last workspace was removed elsewhere) gets here. */
export function NoWorkspace() {
  return (
    <div className="grid h-full place-items-center p-6">
      <div className="grid w-[340px] max-w-full gap-2 rounded-card border border-line p-3">
        <p className="m-0 text-[13px] text-ink-2">A window shows one workspace, and none exists yet. Name your first workspace to start, or open Setup to go through every step.</p>
        <NewWorkspaceForm onDone={() => undefined} onCancel={() => undefined} />
        <Link to="/setup" className="btn btn-sm justify-self-start no-underline">Open Setup</Link>
      </div>
    </div>
  );
}
