// Add folder: the one way to register folders. Pick a machine, browse to any folder (Git or not), tick
// folders (Git repositories of the folders root are suggested), or clone a repository. Every folder belongs to
// exactly one workspace, chosen here (the window's by default).

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import type { Discovered } from "@/gen/Discovered";
import type { Project } from "@/gen/Project";
import { Icon } from "@/components/Icon";
import { toast, toastError } from "@/components/Toast";
import { Empty, ErrorState, Loading, Tabs, Token } from "@/components/ui";
import { api } from "@/lib/api";
import { usePhone } from "@/lib/phone";
import { useConfig, useProjects, useStatus } from "@/lib/queries";
import { useScope, useWorkspaces } from "@/lib/workspace";
import { WorkspacePicker, defaultWorkspace } from "@/features/workspaces/WorkspacePicker";
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
      <div role="dialog" aria-label="Add folders" className="flex max-h-[calc(86vh/var(--lp-zoom,1))] w-[640px] max-w-full flex-col overflow-hidden rounded-xl border border-line bg-paper shadow-[0_20px_60px_-20px_rgba(23,32,43,.6)]" onClick={(e) => e.stopPropagation()}>
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
  const config = useConfig().data?.config;
  const projects = useProjects().data ?? [];
  const workspaces = useWorkspaces().data ?? [];
  const current = useScope().workspace;
  const role = useStatus().data?.role;
  const preset = useAddFolder((s) => s.preset);
  // Clone first everywhere; Home never holds untracked projects, so it has no Browse.
  const [tab, setTab] = useState<"browse" | "clone" | "plain">("clone");
  const [at, setAt] = useState<string | null>(null);
  const [typed, setTyped] = useState("");
  const [picked, setPicked] = useState<Set<string>>(() => new Set());
  const [chosenId, setChosenId] = useState<string | null>(preset.workspaceId ?? null);
  const [busy, setBusy] = useState(false);
  const browse = useQuery({
    queryKey: ["browse", at],
    queryFn: () => api.get<Browse>(`/api/projects/browse${at ? `?path=${encodeURIComponent(at)}` : ""}`),
    enabled: tab === "browse",
  });
  useEffect(() => { if (browse.data) setTyped(browse.data.path); }, [browse.data]);
  const discover = useQuery({ queryKey: ["discover"], queryFn: () => api.post<Discovered[]>("/api/projects/discover", {}), staleTime: 60_000, enabled: tab === "browse" });
  const registered = useMemo(() => new Set(projects.map((p) => p.path)), [projects]);
  const suggestions = (discover.data ?? []).filter((d) => !d.registered).slice(0, 12);
  const d = browse.data;
  const toggle = (native: string) => setPicked((s) => { const n = new Set(s); if (n.has(native)) n.delete(native); else n.add(native); return n; });
  const chosen = workspaces.find((w) => w.id === (chosenId ?? defaultWorkspace(current, workspaces))) ?? null;
  const add = async () => {
    if (!chosen) return;
    setBusy(true);
    const added: Project[] = [];
    try {
      for (const path of picked) added.push(await api.post<Project>("/api/projects", { path, workspace_id: chosen.id }));
      await qc.invalidateQueries({ queryKey: ["projects"] });
      await qc.invalidateQueries({ queryKey: ["discover"] });
      toast(`Added ${added.length === 1 ? added[0].name : `${added.length} folders`} to ${chosen.name}`);
      onClose();
    } catch (e) {
      toastError(e);
      if (added.length) await qc.invalidateQueries({ queryKey: ["projects"] });
    } finally {
      setBusy(false);
    }
  };
  const row = (native: string, label: string, git: boolean, enter?: () => void) => {
    const known = registered.has(native);
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
        <Tabs label="How to add" className="min-h-[33px]" value={tab} onChange={setTab} items={[{ id: "clone" as const, label: "Clone from GitHub", icon: "branch" as const }, ...(role === "home" ? [] : [{ id: "browse" as const, label: "Browse", icon: "folder" as const }]), { id: "plain" as const, label: "New plain folder", icon: "plus" as const }]} />
        {tab === "plain" ? <NewPlainFolder workspaceId={chosen?.id ?? null} onDone={onClose} /> : tab === "clone" ? (
          <CloneFromGitHub root={config?.projects_root} workspace={chosen} initialRepo={preset.repo} onDone={onClose} />
        ) : (
          <>
            {role === "runner" && <p className="rounded-[6px] bg-attn-soft px-2.5 py-2 text-[12px] text-attn">Folders added here are tracked by Home and backed up to Home's cloud bucket: Git folders send their branches, folders without Git have their files copied.</p>}
            {suggestions.length > 0 && (
              <section className="grid gap-1">
                <div className="seclabel">Git repositories in the folders root</div>
                <div className="grid max-h-[200px] overflow-y-auto rounded-lg border border-line">{suggestions.map((s) => row(s.path, s.gh_repo ? `${s.name} · ${s.gh_repo}` : s.name, true))}</div>
              </section>
            )}
            <section className="grid gap-1.5">
              <div className="seclabel">Any folder</div>
              <div className="flex flex-wrap gap-1.5">
                <input className="field-input min-w-0 flex-1 basis-[260px] font-mono text-[12px]" aria-label="Folder path" placeholder="~/work/some-folder" value={typed} onChange={(e) => setTyped(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter" && typed.trim()) setAt(typed.trim()); }} />
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
        <WorkspacePicker label="Add to" value={chosen?.id ?? null} onChange={setChosenId} />
      </div>
      {tab === "browse" && (
        <div className="flex items-center gap-2 border-t border-line-2 px-4 py-2.5">
          <span className="min-w-0 flex-1 truncate text-[12px] text-ink-3">{!chosen ? "Choose a workspace." : picked.size ? [...picked].map((p) => p.split("/").pop()).join(", ") : "Tick the folders to add."}</span>
          <button className="btn" onClick={onClose}>Cancel</button>
          <button className="btn btn-primary" disabled={!picked.size || !chosen || busy} onClick={() => void add()}><Icon name="plus" size={14} />{busy ? "Adding…" : `Add ${picked.size || ""} folder${picked.size === 1 ? "" : "s"}`}</button>
        </div>
      )}
    </>
  );
}

/** A plain folder: files without Git, stored by Home with previous versions and trash. Folders without Git
 * added from Browse become plain folders too (their files are copied once). */
function NewPlainFolder({ workspaceId, onDone }: { workspaceId: string | null; onDone: () => void }) {
  const qc = useQueryClient();
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const create = async () => {
    setBusy(true);
    try {
      await api.post<Project>("/api/projects/plain", { name: name.trim(), workspace_id: workspaceId });
      await qc.invalidateQueries({ queryKey: ["projects"] });
      toast(`${name.trim()} created`);
      onDone();
    } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  return <section className="grid gap-1.5">
    <span className="text-[12px] text-ink-3">For projects without Git. Home stores the files and keeps previous versions and deleted files for 30 days; one agent works in it at a time, in a synced copy.</span>
    <div className="flex gap-1.5">
      <input className="field-input min-w-0 flex-1" aria-label="Folder name" placeholder="Folder name" value={name} onChange={(e) => setName(e.target.value)} />
      <button className="btn btn-sm btn-primary" disabled={busy || !name.trim() || !workspaceId} onClick={() => void create()}>Create</button>
    </div>
  </section>;
}
