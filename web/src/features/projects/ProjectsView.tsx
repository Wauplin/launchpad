import { usePhone } from "@/lib/phone";
import { PhoneHeader } from "@/mobile/PhoneHeader";
// Folders: the registered folders of this window's workspace (then the others), and scratch folders.

import { useMutation, useQueryClient } from "@tanstack/react-query";
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
import { setWorkspaceProjects, useScope } from "@/lib/workspace";

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
  const [name, setName] = useState(p.name.replace(/^tmp-\d{4}-/, ""));
  const promote = useMutation({
    mutationFn: () => api.post<Project>(`/api/projects/${p.id}/promote`, { name }),
    onSuccess: (np) => {
      toast(`Promoted to ${np.path}`);
      void qc.invalidateQueries({ queryKey: ["projects"] });
      void qc.invalidateQueries({ queryKey: ["sessions"] });
    },
    onError: toastError,
  });
  const root = p.path.slice(0, p.path.length - p.name.length);
  return (
    <div className="grid min-w-0 gap-2.5 rounded-lg border border-line p-3.5">
      <div className="flex items-center gap-2">
        <HostLabel path={p.path} />
        <span className="min-w-0 truncate font-mono text-[13px] font-semibold" title={p.path}>
          {p.path}
        </span>
        <span className="ml-auto text-xs whitespace-nowrap text-ink-3">created {formatAge(p.created_at)} ago</span>
      </div>
      <div className="text-xs text-ink-3">
        Scratch folder · {open.length ? `${open.length} open agent${open.length > 1 ? "s" : ""}` : "No open agent"} · {mine.length} in total · Promote keeps it as a regular folder
      </div>
      <div className="flex items-center gap-2">
        <div className="flex h-[30px] flex-1 items-center overflow-hidden rounded-[7px] border border-line">
          <span className="flex h-full max-w-[45%] min-w-0 items-center border-r border-line bg-sunk px-2 font-mono text-xs text-ink-3" title={root}><span className="truncate [direction:rtl]">{root}</span></span>
          <input className="h-full min-w-0 flex-1 px-2 font-mono text-xs outline-none" value={name} onChange={(e) => setName(e.target.value)} />
        </div>
        <button className="btn btn-primary" disabled={!name.trim() || promote.isPending} onClick={() => promote.mutate()}>
          <Icon name="up" size={14} />
          Promote
        </button>
      </div>
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


/** Workspaces of a folder, and Remove from workspace / Remove from Launchpad with their consequences. */
function FolderActions({ p }: { p: Project }) {
  const qc = useQueryClient();
  const w = useScope().workspace;
  const inside = !!w?.project_ids.includes(p.id);
  return (
    <Popover viewport align="right" width={300} trigger={({ open, toggle }) => <button className="iconbtn" aria-label={`More actions for ${p.name}`} title="More actions" aria-expanded={open} onClick={toggle}><Icon name="more" size={14} /></button>}>
      {(close) => (
        <div className="grid gap-1 p-1">
          <Link to="/projects/$id" params={{ id: p.id }} search={{ tab: "settings" } as never} className="flex h-8 items-center gap-2 rounded-md px-2 text-[13px] text-ink no-underline hover:bg-sunk" onClick={close}><Icon name="sliders" size={13} />Folder settings</Link>
          {w && (inside
            ? <button className="flex h-8 items-center gap-2 rounded-md px-2 text-left text-[13px] hover:bg-sunk" onClick={() => { close(); void setWorkspaceProjects(w, w.project_ids.filter((id) => id !== p.id)).then(() => toast(`Removed ${p.name} from ${w.name}`), toastError); }}><Icon name="x" size={13} />Remove from {w.name}<span className="ml-auto text-[11px] text-ink-3">kept in Launchpad</span></button>
            : <button className="flex h-8 items-center gap-2 rounded-md px-2 text-left text-[13px] hover:bg-sunk" onClick={() => { close(); void setWorkspaceProjects(w, [...w.project_ids, p.id]).catch(toastError); }}><Icon name="plus" size={13} />Add to {w.name}</button>)}
          <div className="border-t border-line-2 px-2 pt-1.5 pb-0.5">
            <ConfirmButton icon="trash" consequence="Unregisters it; files on disk are kept." confirmLabel="Remove" onConfirm={() => api.delete(`/api/projects/${p.id}`).then(() => { close(); toast(`Removed ${p.name} from Launchpad`); return qc.invalidateQueries({ queryKey: ["projects"] }); }, toastError)}>Remove from Launchpad</ConfirmButton>
          </div>
        </div>
      )}
    </Popover>
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
  const scope = useScope();
  const w = scope.workspace;
  const showAdd = useAddFolder((s) => s.show);
  const regular = projects.filter((p) => !p.is_temp).sort((a, b) => a.name.localeCompare(b.name));
  // In a workspace: its folders first, then the others, which one click adds.
  const mine = w ? regular.filter((p) => scope.hasProject(p.id)) : regular;
  const others = w ? regular.filter((p) => !scope.hasProject(p.id)) : [];
  const temp = projects.filter((p) => p.is_temp && scope.hasProject(p.id));
  const add = <button className="btn btn-primary" onClick={showAdd}><Icon name="plus" size={14} />Add folder</button>;
  const title = w ? `Folders in ${w.name}` : "Folders";
  return (
    <div className="h-full overflow-y-auto">
      {phone ? <PhoneHeader title={title} subtitle={`${mine.length} folder${mine.length === 1 ? "" : "s"}`} primary={add} /> : <PageHeader title={title} meta={`${mine.length} folder${mine.length === 1 ? "" : "s"}${w ? ` · ${others.length} more in Launchpad` : ""}`} actions={add} />}
      <div className="phone-page-content mx-auto grid max-w-[920px] grid-cols-[minmax(0,1fr)] gap-4 p-6">
        {mine.length === 0 ? <Empty icon="folder" action={add}>{w ? `No folder in ${w.name} yet.` : "No folder yet."}</Empty> : <FolderTable folders={mine} />}
        {others.length > 0 && (
          <>
            <div className="seclabel">Other folders</div>
            <FolderTable folders={others} />
          </>
        )}
        {temp.length > 0 && (
          <>
            <div className="seclabel">Scratch folders</div>
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
