import { usePhone } from "@/lib/phone";
import { PhoneHeader } from "@/mobile/PhoneHeader";
// Folders: the registered folders of this window's workspace, its temporary folders, and the Unassigned folders
// card for folders registered before workspaces were required.

import { useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { memo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { MenuItem } from "@/components/Popover";
import { ConfirmButton, Empty, MenuSection, PageHeader, RowActions } from "@/components/ui";
import { useOpenTerminal, useTerminalsHere } from "@/features/terminals/lib";
import { useAddFolder } from "./addFolderStore";
import type { Project } from "@/gen/Project";
import { Icon } from "@/components/Icon";
import { toast, toastError } from "@/components/Toast";
import { LaunchButton } from "@/features/launch/LaunchButton";
import { api } from "@/lib/api";
import { ago } from "@/lib/format";
import { useProjects, useSessions } from "@/lib/queries";
import { KeepFolder } from "./KeepFolder";
import { isRunning } from "@/lib/sessionStatus";
import type { Workspace } from "@/gen/Workspace";
import { moveFolder, useScope, useWorkspaces } from "@/lib/workspace";

export function Description({ p }: { p: Project }) {
  const [edit, setEdit] = useState<string | null>(null);
  const qc = useQueryClient();
  const save = async () => {
    try {
      await api.patch(`/api/projects/${p.id}`, { description: edit });
      await qc.invalidateQueries({ queryKey: ["projects"] });
    } catch (e) {
      toastError(e);
    }
    setEdit(null);
  };
  if (edit !== null) {
    return (
      <input
        autoFocus
        className="field-input"
        value={edit}
        placeholder="One line, used when the folder is given as context"
        onChange={(e) => setEdit(e.target.value)}
        onBlur={() => void save()}
        onKeyDown={(e) => {
          if (e.key === "Enter") void save();
          if (e.key === "Escape") setEdit(null);
        }}
      />
    );
  }
  return (
    <button className="flex w-fit max-w-full items-center gap-1 truncate text-left text-[12px] text-ink-3 hover:text-ink" title="Edit the description" onClick={() => setEdit(p.description)}>
      {p.description || <span className="italic">Add a description</span>}
      <Icon name="pencil" size={12} className="flex-none" />
    </button>
  );
}

/** What a temporary folder is: the Folders page intro and the folder page intro of a temporary folder. */
export const TEMP_INTRO = "A temporary folder holds an agent launched without a folder. Home saves its files; it leaves Folders when its last agent is archived, and Home deletes it 30 days later, unless you keep it as a folder.";

/** A temporary folder: a row of the Folders list (`row`) or a box on its own page. Launch and ⋯ (Delete files), then
 * Keep as folder; what a temporary folder is stays in the page intro. */
export function TempProjectCard({ p, row }: { p: Project; row?: boolean }) {
  const qc = useQueryClient();
  const sessions = useSessions().data ?? [];
  const mine = sessions.filter((s) => s.cwd === p.path || s.cwd.startsWith(p.path + "/"));
  const open = mine.filter((s) => s.status !== "closed");
  const running = mine.some(isRunning);
  return (
    <div className={`grid min-w-0 grid-cols-[minmax(0,1fr)] gap-2 px-4 py-3 ${row ? "border-b border-line-2 last:border-0" : "rounded-card border border-line"}`}>
      <div className="flex min-w-0 items-center gap-3">
        <div className="grid min-w-0 flex-1 gap-0.5">
          <span className="truncate font-mono text-[13px] font-semibold" title={p.path}>{p.name.replace(/^tmp-\d{4}-/, "") || p.name}</span>
          <span className="truncate text-[12px] text-ink-3">{open.length ? `${open.length} open agent${open.length > 1 ? "s" : ""}` : "No open agent"} · created {ago(p.created_at)}</span>
        </div>
        <RowActions label={`More actions for ${p.name}`} menuWidth={280} main={<LaunchButton small ctx={{ projectId: p.id }} primary={false} title={`Launch an agent in ${p.name}`} />} more={(close) => <MenuSection><ConfirmButton icon="trash" consequence={`Deletes ${p.path} from disk${open.length ? " and stops its agents" : ""}. Home keeps a copy of its files 30 days, for a resume.`} confirmLabel="Delete files" onConfirm={() => api.delete(`/api/projects/${p.id}`).then(() => {
          close();
          toast(`Deleted ${p.path}`);
          void qc.invalidateQueries({ queryKey: ["projects"] });
        }, toastError)}>Delete files</ConfirmButton></MenuSection>} />
      </div>
      <KeepFolder p={p} disabled={running} title={running ? "Wait for its agent to finish" : undefined} />
    </div>
  );
}


/** The ⋯ menu of a folder row: Files, Terminal, Folder settings, Move to another workspace, and Remove with its consequence. */
function FolderMenu({ p, close }: { p: Project; close: () => void }) {
  const qc = useQueryClient();
  const workspaces = useWorkspaces().data ?? [];
  const others = workspaces.filter((w) => w.id !== p.workspace_id);
  const terminals = useTerminalsHere();
  const openTerminal = useOpenTerminal();
  const navigate = useNavigate();
  return (
    <>
      <MenuItem icon={<Icon name="folder" size={14} />} label="Files" onClick={() => { close(); void navigate({ to: "/projects/$id/code", params: { id: p.id } }); }} />
      {terminals && <MenuItem icon={<Icon name="term" size={14} />} label="Terminal" detail="A shell in this folder" onClick={() => { close(); void openTerminal.folder(p.path); }} />}
      <MenuItem icon={<Icon name="sliders" size={14} />} label="Folder settings" onClick={() => { close(); void navigate({ to: "/projects/$id", params: { id: p.id }, search: { tab: "settings" } as never }); }} />
      {others.map((w) => (
        <MenuItem key={w.id} icon={<Icon name="folder" size={14} />} label={`Move to ${w.name}`} onClick={() => { close(); void moveFolder(p, w).then(() => { toast(`Moved ${p.name} to ${w.name}`); return qc.invalidateQueries({ queryKey: ["projects"] }); }, toastError); }} />
      ))}
      <MenuSection>
        <ConfirmButton icon="trash" consequence="Unregisters it; files on disk are kept." confirmLabel="Remove" onConfirm={() => api.delete(`/api/projects/${p.id}`).then(() => { close(); toast(`Removed ${p.name} from Launchpad`); return qc.invalidateQueries({ queryKey: ["projects"] }); }, toastError)}>Remove from Launchpad</ConfirmButton>
      </MenuSection>
    </>
  );
}

/** Folders without a workspace: assign each to one, or remove it. Shown in every view until none is left. */
function UnassignedFolders({ folders, workspaces }: { folders: Project[]; workspaces: Workspace[] }) {
  const qc = useQueryClient();
  const assign = (p: Project, w: Workspace) => moveFolder(p, w).then(() => { toast(`Added ${p.name} to ${w.name}`); return qc.invalidateQueries({ queryKey: ["projects"] }); }, toastError);
  return (
    <section className="grid gap-2 rounded-card border border-attn/40 bg-attn-soft/40 p-3.5" aria-label="Unassigned folders">
      <div className="grid gap-0.5">
        <b className="text-[13px]">Unassigned folders · {folders.length}</b>
        <span className="text-[12px] text-ink-3">Every folder belongs to one workspace. {workspaces.length ? "Choose one for each, or remove the folders you no longer use." : "Create a workspace from the workspace menu, then choose it here."}</span>
      </div>
      <div className="grid max-h-[320px] overflow-y-auto rounded-control border border-line bg-paper">
        {folders.map((p) => (
          <div key={p.id} className="flex min-h-[44px] flex-wrap items-center gap-2 border-b border-line-2 px-3 py-1.5 text-[13px] last:border-0">
            <span className="grid min-w-0 flex-1 basis-[200px]">
              <span className="flex min-w-0 items-center gap-2"><span className="truncate font-semibold">{p.name}</span>{p.is_temp && <span className="text-[11px] text-ink-3">temporary</span>}</span>
              <span className="truncate font-mono text-[12px] text-ink-3" title={p.path}>{p.path}</span>
            </span>
            {workspaces.map((w) => <button key={w.id} className="btn btn-sm" onClick={() => void assign(p, w)}>{w.name}</button>)}
            <ConfirmButton icon="trash" consequence={p.is_temp ? `Deletes ${p.path} from disk.` : "Unregisters it; files on disk are kept."} confirmLabel={p.is_temp ? "Delete files" : "Remove"} onConfirm={() => api.delete(`/api/projects/${p.id}`).then(() => { toast(p.is_temp ? `Deleted ${p.path}` : `Removed ${p.name}`); return qc.invalidateQueries({ queryKey: ["projects"] }); }, toastError)}>{p.is_temp ? "Delete files" : "Remove"}</ConfirmButton>
          </div>
        ))}
      </div>
    </section>
  );
}

const FolderRow = memo(function FolderRow({ p }: { p: Project }) {
  const phone = usePhone();
  return (
    <div className="group flex h-[56px] items-center gap-3 border-b border-line-2 px-4">
      <div className="grid min-w-0 flex-1 gap-0.5">
        <span className="flex min-w-0 items-center gap-2">
          <Link to="/projects/$id" params={{ id: p.id }} className="truncate font-semibold text-ink no-underline hover:underline">{p.name}</Link>
          {p.gh_repo && !phone && <span className="truncate font-mono text-[12px] text-ink-3">{p.gh_repo}</span>}
        </span>
        <span className={`truncate text-[12px] text-ink-3 ${p.description ? "" : "font-mono"}`} title={p.path}>{p.description || p.path}</span>
      </div>
      {/* One visible action (Launch); Files, Terminal and the rest in ⋯. */}
      <RowActions label={`More actions for ${p.name}`} menuWidth={280} main={<LaunchButton small primary={false} ctx={{ projectId: p.id }} title={`Launch an agent in ${p.name}`} />} more={(close) => <FolderMenu p={p} close={close} />} />
    </div>
  );
});

function FolderTable({ folders }: { folders: Project[] }) {
  const ref = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({ count: folders.length, getScrollElement: () => ref.current, estimateSize: () => 56, overscan: 6, useFlushSync: false });
  return (
    <div ref={ref} className="overflow-y-auto rounded-card border border-line" style={{ maxHeight: 12 * 56 + 2 }}>
      <div className="relative" style={{ height: v.getTotalSize() }}>
        {v.getVirtualItems().map((it) => <div key={folders[it.index].id} className="absolute inset-x-0" style={{ top: it.start }}><FolderRow p={folders[it.index]} /></div>)}
      </div>
    </div>
  );
}

export function ProjectsView() {
  const phone = usePhone();
  const projects = useProjects().data ?? [];
  const workspaces = useWorkspaces().data ?? [];
  const scope = useScope();
  const w = scope.workspace;
  const showAdd = useAddFolder((s) => s.show);
  const assigned = projects.filter((p) => p.workspace_id && workspaces.some((x) => x.id === p.workspace_id));
  const unassigned = projects.filter((p) => !assigned.includes(p));
  const regular = assigned.filter((p) => !p.is_temp).sort((a, b) => a.name.localeCompare(b.name));
  const mine = regular.filter((p) => scope.hasProject(p.id));
  const temp = assigned.filter((p) => p.is_temp && scope.hasProject(p.id));
  const add = <button className={phone ? "btn btn-sm" : "btn btn-primary"} data-tour="folders.add" onClick={() => showAdd()}><Icon name="plus" size={phone ? 12 : 14} />Add folder</button>;
  const meta = `${mine.length} folder${mine.length === 1 ? "" : "s"}${temp.length ? ` · ${temp.length} temporary` : ""}`;
  return (
    <div className="h-full overflow-y-auto">
      {phone ? <PhoneHeader title="Folders" primary={add} details={temp.length > 0 && <p className="m-0 text-[13px] text-ink-2">{TEMP_INTRO}</p>} /> : <PageHeader title="Folders" meta={meta} intro={temp.length > 0 ? TEMP_INTRO : undefined} introId="Folders" actions={add} />}
      <div className="phone-page-content grid max-w-page grid-cols-[minmax(0,1fr)] gap-4 px-5 py-5">
        {unassigned.length > 0 && <UnassignedFolders folders={unassigned} workspaces={workspaces} />}
        {mine.length === 0 ? <Empty icon="folder" action={add}>{`No folder in ${w.name} yet.`}</Empty> : <FolderTable folders={mine} />}
        {temp.length > 0 && (
          <>
            <div className="seclabel">Temporary folders</div>
            <div className="grid grid-cols-[minmax(0,1fr)] overflow-hidden rounded-card border border-line">
              {temp.map((p) => (
                <TempProjectCard key={p.id} p={p} row />
              ))}
            </div>
          </>
        )}
      </div>
    </div>
  );
}
