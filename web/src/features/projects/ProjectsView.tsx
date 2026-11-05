import { usePhone } from "@/lib/phone";
import { PhoneHeader } from "@/mobile/PhoneHeader";
// Folders: the registered folders of this window's workspace (Everything: every workspace, one section each),
// temporary folders, and the Unassigned folders card for folders registered before workspaces were required.

import { useQueryClient } from "@tanstack/react-query";
import { HostLabel } from "@/components/HostLabel";
import { Link } from "@tanstack/react-router";
import { memo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Popover } from "@/components/Popover";
import { ConfirmButton, Empty, PageHeader } from "@/components/ui";
import { TerminalMenu } from "@/features/terminals/TerminalMenu";
import { useAddFolder } from "./addFolderStore";
import type { Project } from "@/gen/Project";
import { Icon } from "@/components/Icon";
import { toast, toastError } from "@/components/Toast";
import { LaunchButton } from "@/features/launch/LaunchButton";
import { api } from "@/lib/api";
import { formatAge } from "@/lib/format";
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
        className="field-input h-7 text-xs"
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
    <button className="flex w-fit max-w-full items-center gap-1 truncate text-left text-xs text-ink-3 hover:text-ink" title="Edit the description" onClick={() => setEdit(p.description)}>
      {p.description || <span className="italic">Add a description</span>}
      <Icon name="pencil" size={11} className="flex-none" />
    </button>
  );
}

export function TempProjectCard({ p }: { p: Project }) {
  const qc = useQueryClient();
  const sessions = useSessions().data ?? [];
  const mine = sessions.filter((s) => s.cwd === p.path || s.cwd.startsWith(p.path + "/"));
  const open = mine.filter((s) => s.status !== "closed");
  const running = mine.some(isRunning);
  return (
    <div className="grid min-w-0 grid-cols-[minmax(0,1fr)] gap-2.5 rounded-lg border border-line p-3.5">
      <div className="flex items-center gap-2">
        <HostLabel path={p.path} />
        <span className="min-w-0 truncate font-mono text-[13px] font-semibold" title={p.path}>
          {p.name.replace(/^tmp-\d{4}-/, "") || p.name}
        </span>
        <span className="ml-auto text-xs whitespace-nowrap text-ink-3">created {formatAge(p.created_at)} ago</span>
      </div>
      <div className="text-xs text-ink-3">
        Temporary folder · {open.length ? `${open.length} open agent${open.length > 1 ? "s" : ""}` : "No open agent"} · Deleted when its last agent is archived, unless you keep it as a folder
      </div>
      <KeepFolder p={p} disabled={running} title={running ? "Wait for its agent to finish" : undefined} />
      <div className="flex items-center gap-2">
        <LaunchButton small ctx={{ projectId: p.id }} label="Launch here" primary={false} />
        <ConfirmButton icon="trash" consequence={`Deletes ${p.path} from disk${open.length ? " and stops its agents" : ""}.`} confirmLabel="Delete files" onConfirm={() => api.delete(`/api/projects/${p.id}`).then(() => {
          toast(`Deleted ${p.path}`);
          void qc.invalidateQueries({ queryKey: ["projects"] });
        }, toastError)}>Delete files</ConfirmButton>
      </div>
    </div>
  );
}


/** Folder settings, Move to another workspace, and Remove from Launchpad with its consequence. */
function FolderActions({ p }: { p: Project }) {
  const qc = useQueryClient();
  const workspaces = useWorkspaces().data ?? [];
  const others = workspaces.filter((w) => w.id !== p.workspace_id);
  return (
    <Popover viewport align="right" width={300} trigger={({ open, toggle }) => <button className="iconbtn" aria-label={`More actions for ${p.name}`} title="More actions" aria-expanded={open} onClick={toggle}><Icon name="more" size={14} /></button>}>
      {(close) => (
        <div className="grid gap-1 p-1">
          <Link to="/projects/$id" params={{ id: p.id }} search={{ tab: "settings" } as never} className="flex h-8 items-center gap-2 rounded-md px-2 text-[13px] text-ink no-underline hover:bg-sunk" onClick={close}><Icon name="sliders" size={13} />Folder settings</Link>
          {others.map((w) => (
            <button key={w.id} className="flex h-8 items-center gap-2 rounded-md px-2 text-left text-[13px] hover:bg-sunk" onClick={() => { close(); void moveFolder(p, w).then(() => { toast(`Moved ${p.name} to ${w.name}`); return qc.invalidateQueries({ queryKey: ["projects"] }); }, toastError); }}><Icon name="folder" size={13} />Move to {w.name}</button>
          ))}
          <div className="border-t border-line-2 px-2 pt-1.5 pb-0.5">
            <ConfirmButton icon="trash" consequence="Unregisters it; files on disk are kept." confirmLabel="Remove" onConfirm={() => api.delete(`/api/projects/${p.id}`).then(() => { close(); toast(`Removed ${p.name} from Launchpad`); return qc.invalidateQueries({ queryKey: ["projects"] }); }, toastError)}>Remove from Launchpad</ConfirmButton>
          </div>
        </div>
      )}
    </Popover>
  );
}

/** Folders without a workspace: assign each to one, or remove it. Shown in every view until none is left. */
function UnassignedFolders({ folders, workspaces }: { folders: Project[]; workspaces: Workspace[] }) {
  const qc = useQueryClient();
  const assign = (p: Project, w: Workspace) => moveFolder(p, w).then(() => { toast(`Added ${p.name} to ${w.name}`); return qc.invalidateQueries({ queryKey: ["projects"] }); }, toastError);
  return (
    <section className="grid gap-2 rounded-lg border border-attn/40 bg-attn-soft/40 p-3.5" aria-label="Unassigned folders">
      <div className="grid gap-0.5">
        <b className="text-[13px]">Unassigned folders · {folders.length}</b>
        <span className="text-[12px] text-ink-3">Every folder belongs to one workspace. {workspaces.length ? "Choose one for each, or remove the folders you no longer use." : "Create a workspace from the sidebar menu, then choose it here."}</span>
      </div>
      <div className="grid max-h-[320px] overflow-y-auto rounded-md border border-line bg-paper">
        {folders.map((p) => (
          <div key={p.id} className="flex min-h-[44px] flex-wrap items-center gap-2 border-b border-line-2 px-3 py-1.5 text-[13px] last:border-0">
            <span className="grid min-w-0 flex-1 basis-[200px]">
              <span className="flex min-w-0 items-center gap-2"><span className="truncate font-semibold">{p.name}</span><HostLabel path={p.path} />{p.is_temp && <span className="text-[11px] text-ink-3">temporary</span>}</span>
              <span className="truncate font-mono text-[12px] text-ink-3" title={p.path}>{p.path}</span>
            </span>
            {workspaces.map((w) => <button key={w.id} className="btn btn-sm" onClick={() => void assign(p, w)}>{w.name}</button>)}
            <ConfirmButton icon="trash" consequence={p.is_temp ? `Deletes ${p.path} from disk.` : "Unregisters it; files on disk are kept."} confirmLabel={p.is_temp ? "Delete files" : "Remove"} onConfirm={() => api.delete(`/api/projects/${p.id}`).then(() => { toast(`Removed ${p.name}`); return qc.invalidateQueries({ queryKey: ["projects"] }); }, toastError)}>Remove</ConfirmButton>
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
          <HostLabel path={p.path} />
          {p.gh_repo && !phone && <span className="truncate font-mono text-[12px] text-ink-3">{p.gh_repo}</span>}
        </span>
        <span className="truncate font-mono text-[12px] text-ink-3" title={p.path}>{p.description || p.path}</span>
      </div>
      <Link to="/projects/$id/code" params={{ id: p.id }} className="btn btn-sm no-underline" title={`Browse the files of ${p.name}`}><Icon name="folder" size={12} />{!phone && "Files"}</Link>
      {!phone && <TerminalMenu cwd={p.path} small viewport />}
      <LaunchButton small ctx={{ projectId: p.id }} label={null} title={`Launch an agent in ${p.name}`} />
      <FolderActions p={p} />
    </div>
  );
});

function FolderTable({ folders }: { folders: Project[] }) {
  const ref = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({ count: folders.length, getScrollElement: () => ref.current, estimateSize: () => 56, overscan: 6, useFlushSync: false });
  return (
    <div ref={ref} className="overflow-y-auto rounded-lg border border-line" style={{ maxHeight: 12 * 56 + 2 }}>
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
  const mine = w ? regular.filter((p) => scope.hasProject(p.id)) : regular;
  // Everything is a view: one section per workspace.
  const sections = w ? [{ w, folders: mine }] : workspaces.map((x) => ({ w: x, folders: regular.filter((p) => p.workspace_id === x.id) })).filter((x) => x.folders.length > 0);
  const temp = assigned.filter((p) => p.is_temp && (!w || scope.hasProject(p.id)));
  const add = <button className="btn btn-primary" onClick={showAdd}><Icon name="plus" size={14} />Add folder</button>;
  const title = w ? `Folders in ${w.name}` : "Folders";
  return (
    <div className="h-full overflow-y-auto">
      {phone ? <PhoneHeader title={title} subtitle={`${mine.length} folder${mine.length === 1 ? "" : "s"}`} primary={add} /> : <PageHeader title={title} meta={`${mine.length} folder${mine.length === 1 ? "" : "s"}${w ? "" : ` · ${workspaces.length} workspace${workspaces.length === 1 ? "" : "s"}`}`} actions={add} />}
      <div className="phone-page-content mx-auto grid max-w-[920px] grid-cols-[minmax(0,1fr)] gap-4 p-6">
        {unassigned.length > 0 && <UnassignedFolders folders={unassigned} workspaces={workspaces} />}
        {mine.length === 0 ? <Empty icon="folder" action={add}>{w ? `No folder in ${w.name} yet.` : "No folder yet."}</Empty> : sections.map((x) => (
          <section key={x.w.id} className="grid gap-2">
            {!w && <div className="seclabel">{x.w.name}</div>}
            <FolderTable folders={x.folders} />
          </section>
        ))}
        {temp.length > 0 && (
          <>
            <div className="seclabel">Temporary folders</div>
            <div className="grid grid-cols-[minmax(0,1fr)] gap-3">
              {temp.map((p) => (
                <TempProjectCard key={p.id} p={p} />
              ))}
            </div>
          </>
        )}
      </div>
    </div>
  );
}
