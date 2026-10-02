// Projects: the registry, discovery in the projects root, and temporary projects (sketch S6).

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useState } from "react";
import type { Discovered } from "@/gen/Discovered";
import type { Project } from "@/gen/Project";
import { Icon } from "@/components/Icon";
import { toast, toastError } from "@/components/Toast";
import { LaunchButton } from "@/features/launch/Radial";
import { api } from "@/lib/api";
import { formatAge } from "@/lib/format";
import { useConfig, useProjects, useSessions } from "@/lib/queries";

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
        placeholder="One line, used when the project is given as context"
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
    <button className="truncate text-left text-xs text-ink-3 hover:text-ink" title="Edit the description" onClick={() => setEdit(p.description)}>
      {p.description || <span className="italic">Add a description</span>}
    </button>
  );
}

function PinButton({ p }: { p: Project }) {
  const qc = useQueryClient();
  return (
    <button
      className={`btn btn-sm ${p.pinned ? "border-teal! text-teal-ink" : ""}`}
      title={p.pinned ? "Unpin from the sidebar" : "Pin to the sidebar"}
      onClick={() => void api.patch(`/api/projects/${p.id}`, { pinned: !p.pinned }).then(() => qc.invalidateQueries({ queryKey: ["projects"] }), toastError)}
    >
      {p.pinned ? "Pinned" : "Pin"}
    </button>
  );
}

function ProjectRow({ p }: { p: Project }) {
  const qc = useQueryClient();
  const [confirm, setConfirm] = useState(false);
  return (
    <div className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-3 border-b border-line-2 px-4 py-2.5">
      <div className="grid min-w-0 gap-0.5">
        <span className="flex items-center gap-2">
          <Link to="/projects/$id" params={{ id: p.id }} className="font-semibold text-ink no-underline hover:underline">
            {p.name}
          </Link>
          {p.gh_repo && <span className="truncate font-mono text-[11.5px] text-ink-3">{p.gh_repo}</span>}
        </span>
        <Description p={p} />
      </div>
      <div className="flex items-center gap-1.5">
        <PinButton p={p} />
        <LaunchButton small ctx={{ projectId: p.id }} label={null} />
        <button
          className={`btn btn-sm ${confirm ? "border-[#A40E26]! bg-gh-closed! text-white!" : "btn-ghost"}`}
          title="Remove from Launchpad (the folder is kept)"
          onMouseLeave={() => setConfirm(false)}
          onClick={() => {
            if (!confirm) return setConfirm(true);
            void api.delete(`/api/projects/${p.id}`).then(() => qc.invalidateQueries({ queryKey: ["projects"] }), toastError);
          }}
        >
          {confirm ? "Click again to remove" : <Icon name="trash" size={13} />}
        </button>
      </div>
    </div>
  );
}

export function TempProjectCard({ p }: { p: Project }) {
  const qc = useQueryClient();
  const sessions = useSessions().data ?? [];
  const mine = sessions.filter((s) => s.cwd === p.path || s.cwd.startsWith(p.path + "/"));
  const open = mine.filter((s) => s.status !== "closed");
  const [name, setName] = useState(p.name.replace(/^tmp-\d{4}-/, ""));
  const [confirm, setConfirm] = useState(false);
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
        <span className="min-w-0 truncate font-mono text-[13px] font-semibold" title={p.path}>
          {p.path}
        </span>
        <span className="ml-auto text-xs whitespace-nowrap text-ink-3">created {formatAge(p.created_at)} ago</span>
      </div>
      <div className="text-xs text-ink-3">
        {open.length ? `${open.length} open session${open.length > 1 ? "s" : ""}` : "No open session"} · {mine.length} in total
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
        <button
          className={`btn btn-sm ${confirm ? "border-[#A40E26]! bg-gh-closed! text-white!" : ""}`}
          onMouseLeave={() => setConfirm(false)}
          onClick={() => {
            if (!confirm) return setConfirm(true);
            void api.delete(`/api/projects/${p.id}`).then(() => {
              toast(`Deleted ${p.path}`);
              void qc.invalidateQueries({ queryKey: ["projects"] });
            }, toastError);
          }}
        >
          <Icon name="trash" size={13} />
          {confirm ? "Click again to delete the folder" : "Delete folder"}
        </button>
        {open.length > 0 && <span className="text-xs text-ink-3">Stops its sessions first</span>}
      </div>
    </div>
  );
}

function Discover({ onClose }: { onClose: () => void }) {
  const qc = useQueryClient();
  const [filter, setFilter] = useState("");
  const q = useQuery({ queryKey: ["discover"], queryFn: () => api.post<Discovered[]>("/api/projects/discover") });
  const add = async (d: Discovered) => {
    try {
      await api.post("/api/projects", { path: d.path });
      await qc.invalidateQueries({ queryKey: ["projects"] });
      await qc.invalidateQueries({ queryKey: ["discover"] });
    } catch (e) {
      toastError(e);
    }
  };
  const list = (q.data ?? []).filter((d) => !filter || d.name.toLowerCase().includes(filter.toLowerCase()) || d.gh_repo?.toLowerCase().includes(filter.toLowerCase()));
  return (
    <div className="grid gap-2 rounded-lg border border-line bg-chrome p-3">
      <div className="flex items-center gap-2">
        <b className="text-[12.5px]">Git repositories in the projects root</b>
        <input className="field-input ml-auto h-7 w-56 text-xs" placeholder="Filter" value={filter} onChange={(e) => setFilter(e.target.value)} />
        <button className="iconbtn" title="Close" onClick={onClose}>
          <Icon name="x" size={14} />
        </button>
      </div>
      {q.isLoading && <div className="text-xs text-ink-3">Scanning…</div>}
      <div className="grid max-h-[340px] overflow-y-auto rounded-md border border-line bg-paper">
        {list.map((d) => (
          <div key={d.path} className="flex items-center gap-2 border-b border-line-2 px-3 py-1.5 text-[12.5px] last:border-0">
            <span className="font-semibold">{d.name}</span>
            <span className="truncate font-mono text-[11.5px] text-ink-3">{d.gh_repo ?? "no GitHub remote"}</span>
            <span className="ml-auto">
              {d.registered ? (
                <span className="text-xs text-gh-open">Added</span>
              ) : (
                <button className="btn btn-sm" onClick={() => void add(d)}>
                  <Icon name="plus" size={12} />
                  Add
                </button>
              )}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}

export function ProjectsView() {
  const projects = useProjects().data ?? [];
  const config = useConfig().data?.config;
  const qc = useQueryClient();
  const [scan, setScan] = useState(false);
  const [path, setPath] = useState("");
  const regular = projects.filter((p) => !p.is_temp).sort((a, b) => Number(b.pinned) - Number(a.pinned) || a.name.localeCompare(b.name));
  const temp = projects.filter((p) => p.is_temp);
  const addPath = async () => {
    try {
      await api.post("/api/projects", { path });
      setPath("");
      await qc.invalidateQueries({ queryKey: ["projects"] });
    } catch (e) {
      toastError(e);
    }
  };
  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto grid max-w-[920px] grid-cols-[minmax(0,1fr)] gap-4 p-6">
        <div className="flex items-center gap-2">
          <h2 className="m-0 text-lg font-semibold">Projects</h2>
          <span className="min-w-0 truncate text-xs text-ink-3">in {config?.projects_root}</span>
          <button className="btn ml-auto" onClick={() => setScan((v) => !v)}>
            <Icon name="search" size={14} />
            Scan projects root
          </button>
        </div>
        {scan && <Discover onClose={() => setScan(false)} />}
        <div className="overflow-hidden rounded-lg border border-line">
          {regular.length === 0 && <div className="p-4 text-[12.5px] text-ink-2">No project yet. Use "Scan projects root" or add a folder below.</div>}
          {regular.map((p) => (
            <ProjectRow key={p.id} p={p} />
          ))}
          <div className="flex gap-2 bg-chrome px-4 py-2.5">
            <input
              className="field-input h-7 flex-1 font-mono text-xs"
              placeholder="Add a folder by path, for example ~/work/some-repo"
              value={path}
              onChange={(e) => setPath(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && path.trim() && void addPath()}
            />
            <button className="btn btn-sm" disabled={!path.trim()} onClick={() => void addPath()}>
              <Icon name="plus" size={12} />
              Add folder
            </button>
          </div>
        </div>
        {temp.length > 0 && (
          <>
            <div className="seclabel">Temporary</div>
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
