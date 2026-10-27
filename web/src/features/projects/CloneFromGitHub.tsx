// Add folder → Clone from GitHub: the daemon clones a repository into the folders root and registers it.
// Suggestions come from repositories already in the GitHub cache; any owner/name can be typed.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import type { Project } from "@/gen/Project";
import { Icon } from "@/components/Icon";
import { toast, toastError } from "@/components/Toast";
import { api } from "@/lib/api";
import type { Workspace } from "@/gen/Workspace";
import { setWorkspaceProjects } from "@/lib/workspace";

const SHOWN = 30;

/** `addTo`: the workspaces the clone joins, chosen explicitly in the Add folder dialog. */
export function CloneFromGitHub({ root, host = null, addTo, onDone }: { root: string | undefined; host?: string | null; addTo: Workspace[]; onDone?: () => void }) {
  const qc = useQueryClient();
  const [repo, setRepo] = useState("");
  const [cloning, setCloning] = useState<string | null>(null);
  const q = useQuery({ queryKey: ["clone-suggestions"], queryFn: () => api.get<string[]>("/api/projects/clone") });
  const typed = repo.trim();
  const matches = (q.data ?? []).filter((r) => !typed || r.toLowerCase().includes(typed.toLowerCase()));
  const clone = async (name: string) => {
    setCloning(name);
    try {
      const p = await api.post<Project>("/api/projects/clone", { repo: name, host, workspace_id: null });
      for (const w of addTo) if (!w.project_ids.includes(p.id)) await setWorkspaceProjects(w, [...w.project_ids, p.id]);
      toast(`Cloned ${name} into ${p.path}${addTo.length ? ` and added it to ${addTo.map((w) => w.name).join(", ")}` : ""}`);
      onDone?.();
      setRepo("");
      await qc.invalidateQueries({ queryKey: ["projects"] });
      await qc.invalidateQueries({ queryKey: ["clone-suggestions"] });
      await qc.invalidateQueries({ queryKey: ["discover"] });
    } catch (e) {
      toastError(e);
    } finally {
      setCloning(null);
    }
  };
  return (
    <div className="grid gap-2">
      <div className="text-[12px] text-ink-3">Clones into <span className="font-mono">{root ?? "the folders root"}</span>{addTo.length ? `, then adds it to ${addTo.map((w) => w.name).join(", ")}` : ", then adds it to Launchpad"}.</div>
      <div className="flex gap-2">
        <input className="field-input h-7 flex-1 font-mono text-xs" aria-label="Repository" placeholder="owner/name, or filter the repositories below" value={repo} disabled={!!cloning} onChange={(e) => setRepo(e.target.value)} />
        <button className="btn btn-sm btn-primary" disabled={!typed.includes("/") || !!cloning} onClick={() => void clone(typed)}>
          {cloning ? "Cloning…" : "Clone"}
        </button>
      </div>
      {cloning && <div className="text-xs text-ink-2">Cloning {cloning} on {host ?? "the daemon host"}. Large repositories can take a few minutes.</div>}
      <p className="text-xs text-ink-3">Uses <code className="font-mono">gh repo clone</code> with the GitHub CLI sign-in and Git protocol of {host ?? "the daemon host"} (<code className="font-mono">git clone</code> over HTTPS without the GitHub CLI).</p>
      {q.isLoading && <div className="text-xs text-ink-3">Loading cached repositories…</div>}
      {q.data && q.data.length === 0 && <div className="text-xs text-ink-3">No other repository in the GitHub cache yet. Type owner/name.</div>}
      {matches.length > 0 && (
        <div className="grid rounded-md border border-line bg-paper">
          {matches.slice(0, SHOWN).map((r) => (
            <div key={r} className="flex h-9 items-center gap-2 border-b border-line-2 px-3 text-[13px] last:border-0">
              <span className="truncate font-mono text-[12px]">{r}</span>
              <button className="btn btn-sm ml-auto" disabled={!!cloning} onClick={() => void clone(r)}>
                <Icon name="plus" size={12} />
                Clone
              </button>
            </div>
          ))}
          {matches.length > SHOWN && <div className="px-3 py-1.5 text-xs text-ink-3">{matches.length - SHOWN} more; type to filter.</div>}
        </div>
      )}
    </div>
  );
}
