// Add folder: the one way to register folders. Pick a machine, browse to any folder (Git or not), tick
// folders (Git repositories of the folders root are suggested), choose the workspaces they join, or clone a
// repository. Membership is explicit here; nothing joins a workspace silently.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useEffect, useMemo, useState } from "react";
import type { Discovered } from "@/gen/Discovered";
import type { Project } from "@/gen/Project";
import { Icon } from "@/components/Icon";
import { toast, toastError } from "@/components/Toast";
import { Empty, ErrorState, FilterChip, FilterChips, Loading, Tabs, Token } from "@/components/ui";
import { api } from "@/lib/api";
import { useHosts } from "@/lib/hosts";
import { usePhone } from "@/lib/phone";
import { useConfig, useProjects } from "@/lib/queries";
import { setWorkspaceProjects, useScope, useWorkspaces } from "@/lib/workspace";
import { Sheet } from "@/mobile/Sheet";
import { CloneFromGitHub } from "./CloneFromGitHub";
import { useAddFolder } from "./addFolderStore";

type Browse = { path: string; location: string; parent: string | null; home: string; git: boolean; entries: { name: string; git?: boolean }[] };

const join = (dir: string, name: string) => `${dir.replace(/\/$/, "")}/${name}`;

/** Mounted once per shell; opened by `useAddFolder.getState().show()`. */
export function AddFolderDialog() {
  const open = useAddFolder((s) => s.open);
  const close = useAddFolder((s) => s.close);
  const phone = usePhone();
  if (!open) return null;
  if (phone) return <Sheet title="Add folders" full onClose={close}><Body onClose={close} /></Sheet>;
  return (
    <div className="fixed inset-0 z-50 grid place-items-center bg-black/35 p-6" onClick={close}>
      <div role="dialog" aria-label="Add folders" className="flex max-h-[86vh] w-[640px] max-w-full flex-col overflow-hidden rounded-xl border border-line bg-paper shadow-[0_20px_60px_-20px_rgba(23,32,43,.6)]" onClick={(e) => e.stopPropagation()}>
        <div className="flex items-center gap-2 border-b border-line-2 px-4 py-2.5">
          <b className="flex-1 text-[15px]">Add folders</b>
          <button className="iconbtn" aria-label="Close" title="Close" onClick={close}><Icon name="x" size={14} /></button>
        </div>
        <Body onClose={close} />
      </div>
    </div>
  );
}

function Body({ onClose }: { onClose: () => void }) {
  const qc = useQueryClient();
  const hosts = useHosts();
  const config = useConfig().data?.config;
  const projects = useProjects().data ?? [];
  const workspaces = useWorkspaces().data ?? [];
  const current = useScope().workspace;
  const [tab, setTab] = useState<"browse" | "clone">("browse");
  const [machine, setMachine] = useState<string | null>(null);
  const host = hosts.some((h) => h.host.id === machine) ? machine : null;
  const hostInfo = hosts.find((h) => h.host.id === host);
  const [at, setAt] = useState<string | null>(null);
  const [typed, setTyped] = useState("");
  const [picked, setPicked] = useState<Set<string>>(() => new Set());
  const [addTo, setAddTo] = useState<Set<string>>(() => new Set(current ? [current.id] : []));
  const [busy, setBusy] = useState(false);
  useEffect(() => { setAt(null); setPicked(new Set()); }, [host]);
  const browse = useQuery({
    queryKey: ["browse", host, at],
    queryFn: () => api.get<Browse>(`${host ? `/api/hosts/${host}/browse` : "/api/projects/browse"}${at ? `?path=${encodeURIComponent(at)}` : ""}`),
  });
  useEffect(() => { if (browse.data) setTyped(browse.data.path); }, [browse.data]);
  const discover = useQuery({ queryKey: ["discover", host], queryFn: () => api.post<Discovered[]>("/api/projects/discover", { host }), staleTime: 60_000 });
  // A folder is known by its location (`host:/path` on hosts), the same form as Project.path.
  const location = (native: string) => (host ? `${host}:${native}` : native);
  const registered = useMemo(() => new Set(projects.map((p) => p.path)), [projects]);
  const suggestions = (discover.data ?? []).filter((d) => !d.registered).slice(0, 12);
  const d = browse.data;
  const toggle = (native: string) => setPicked((s) => { const n = new Set(s); if (n.has(native)) n.delete(native); else n.add(native); return n; });
  const nativeOf = (path: string) => (host && path.startsWith(`${host}:`) ? path.slice(host.length + 1) : path);
  const chosen = workspaces.filter((w) => addTo.has(w.id));
  const add = async () => {
    setBusy(true);
    const added: Project[] = [];
    try {
      for (const path of picked) added.push(await api.post<Project>("/api/projects", { path, host, workspace_id: null }));
      for (const w of chosen) {
        const ids = [...w.project_ids, ...added.map((p) => p.id).filter((id) => !w.project_ids.includes(id))];
        if (ids.length !== w.project_ids.length) await setWorkspaceProjects(w, ids);
      }
      await qc.invalidateQueries({ queryKey: ["projects"] });
      await qc.invalidateQueries({ queryKey: ["discover"] });
      toast(`Added ${added.length === 1 ? added[0].name : `${added.length} folders`}${chosen.length ? ` to ${chosen.map((w) => w.name).join(", ")}` : ""}`);
      onClose();
    } catch (e) {
      toastError(e);
      if (added.length) await qc.invalidateQueries({ queryKey: ["projects"] });
    } finally {
      setBusy(false);
    }
  };
  const row = (native: string, label: string, git: boolean, enter?: () => void) => {
    const known = registered.has(location(native));
    return (
      <div key={native} className="flex h-10 items-center gap-2 border-b border-line-2 px-3 text-[13px] last:border-0">
        <input type="checkbox" className="accent-teal" aria-label={`Add ${label}`} disabled={known} checked={known || picked.has(native)} onChange={() => toggle(native)} />
        <button className="flex min-w-0 flex-1 items-center gap-2 text-left hover:underline disabled:no-underline" disabled={!enter} onClick={enter} title={enter ? `Open ${native}` : native}>
          <Icon name="folder" size={13} className="flex-none text-ink-3" />
          <span className="truncate">{label}</span>
        </button>
        {git && <Token icon="branch">Git</Token>}
        {known && <span className="text-[12px] text-gh-open">Added</span>}
      </div>
    );
  };
  return (
    <>
      <div className="grid min-h-0 flex-1 content-start gap-3 overflow-y-auto p-4">
        <Tabs label="How to add" value={tab} onChange={setTab} items={[{ id: "browse", label: "Browse", icon: "folder" }, { id: "clone", label: "Clone from GitHub", icon: "branch" }]} />
        <FilterChips label="Machine">
          <FilterChip active={host === null} onClick={() => setMachine(null)}>This computer</FilterChip>
          {hosts.map((h) => <FilterChip key={h.host.id} active={host === h.host.id} title={h.state === "connected" ? h.host.destination : h.error ?? h.state} onClick={() => setMachine(h.host.id)}>{h.host.id}{h.state !== "connected" && <span className="text-danger-ink">·</span>}</FilterChip>)}
          <Link to="/settings/hosts" className="lp-chip no-underline" onClick={onClose}><Icon name="plus" size={11} />New host…</Link>
        </FilterChips>
        {tab === "clone" ? (
          <CloneFromGitHub key={host ?? "local"} host={host} root={host ? `${host}:${hostInfo?.host.projects_root ?? ""}` : config?.projects_root} addTo={chosen} onDone={onClose} />
        ) : (
          <>
            {suggestions.length > 0 && (
              <section className="grid gap-1">
                <div className="seclabel">Git repositories in the folders root</div>
                <div className="grid max-h-[200px] overflow-y-auto rounded-lg border border-line">{suggestions.map((s) => row(nativeOf(s.path), s.gh_repo ? `${s.name} · ${s.gh_repo}` : s.name, true))}</div>
              </section>
            )}
            <section className="grid gap-1.5">
              <div className="seclabel">Any folder</div>
              <div className="flex flex-wrap gap-1.5">
                <input className="field-input h-[30px] min-w-0 flex-1 basis-[260px] font-mono text-xs" aria-label="Folder path" placeholder={host ? "/home/me/work or ~/work" : "~/work/some-folder"} value={typed} onChange={(e) => setTyped(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter" && typed.trim()) setAt(typed.trim()); }} />
                <button className="btn btn-sm" disabled={!typed.trim()} onClick={() => setAt(typed.trim())}>Go</button>
                {d?.parent && <button className="btn btn-sm" onClick={() => setAt(d.parent)}><Icon name="up" size={12} />Up</button>}
                <button className="btn btn-sm" onClick={() => setAt("~")}>Home</button>
              </div>
              {browse.isLoading ? <Loading what="folders" /> : browse.error ? <ErrorState error={browse.error} /> : d && (
                <div className="grid max-h-[300px] overflow-y-auto rounded-lg border border-line">
                  {row(d.path, `This folder · ${d.path.split("/").pop() || d.path}`, d.git)}
                  {d.entries.length === 0 ? <Empty icon="folder">No subfolder.</Empty> : d.entries.map((e) => row(join(d.path, e.name), e.name, !!e.git, () => setAt(join(d.path, e.name))))}
                </div>
              )}
            </section>
          </>
        )}
        {workspaces.length > 0 && (
          <FilterChips label="Add to workspaces">
            <span className="text-[12px] text-ink-3">Add to</span>
            {workspaces.map((w) => <FilterChip key={w.id} active={addTo.has(w.id)} onClick={() => setAddTo((s) => { const n = new Set(s); if (n.has(w.id)) n.delete(w.id); else n.add(w.id); return n; })}>{addTo.has(w.id) && <Icon name="check" size={11} />}{w.name}</FilterChip>)}
          </FilterChips>
        )}
      </div>
      {tab === "browse" && (
        <div className="flex items-center gap-2 border-t border-line-2 px-4 py-2.5">
          <span className="min-w-0 flex-1 truncate text-[12px] text-ink-3">{picked.size ? [...picked].map((p) => p.split("/").pop()).join(", ") : "Tick the folders to add."}</span>
          <button className="btn" onClick={onClose}>Cancel</button>
          <button className="btn btn-primary" disabled={!picked.size || busy} onClick={() => void add()}><Icon name="plus" size={14} />{busy ? "Adding…" : `Add ${picked.size || ""} folder${picked.size === 1 ? "" : "s"}`}</button>
        </div>
      )}
    </>
  );
}
