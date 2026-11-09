// Add folder → Clone from GitHub: the daemon clones a repository into the folders root and registers it.
// Suggestions put repositories with recent GitHub notifications first, then recently pushed ones; any owner/name can be typed.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import type { CloneSuggestion } from "@/gen/CloneSuggestion";
import type { Project } from "@/gen/Project";
import { Icon } from "@/components/Icon";
import { toast, toastError } from "@/components/Toast";
import { api } from "@/lib/api";
import type { Workspace } from "@/gen/Workspace";

const SHOWN = 30;

/** `workspace`: the one workspace of the new folder, chosen in the Add folder dialog (null: not chosen yet).
 * `initialRepo`: a repository to fill in (Settings → GitHub → Add to). */
export function CloneFromGitHub({ root, workspace, initialRepo, onDone }: { root: string | undefined; workspace: Workspace | null; initialRepo?: string; onDone?: () => void }) {
  const qc = useQueryClient();
  const [repo, setRepo] = useState(initialRepo ?? "");
  const [cloning, setCloning] = useState<string | null>(null);
  // POST refreshes recent notifications and pushed repositories from GitHub (best effort), then reads the cache.
  const q = useQuery({ queryKey: ["clone-suggestions"], queryFn: () => api.post<CloneSuggestion[]>("/api/projects/clone/suggestions", {}) });
  const typed = repo.trim();
  const matches = (q.data ?? []).filter((r) => !typed || r.repo.toLowerCase().includes(typed.toLowerCase()));
  const clone = async (name: string) => {
    if (!workspace) return;
    setCloning(name);
    try {
      const p = await api.post<Project>("/api/projects/clone", { repo: name, workspace_id: workspace.id });
      toast(`Cloned ${name} into ${p.path} and added it to ${workspace.name}`);
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
      <div className="text-[12px] text-ink-3">Clones into <span className="font-mono">{root ?? "the folders root"}</span>{workspace ? `, then adds it to ${workspace.name}` : ". Choose its workspace below"}. A repository already cloned for another workspace gets a second folder.</div>
      <div className="flex gap-2">
        <input className="field-input flex-1 font-mono text-[12px]" aria-label="Repository" placeholder="owner/name, or filter the repositories below" value={repo} disabled={!!cloning} onChange={(e) => setRepo(e.target.value)} />
        <button className="btn btn-sm btn-primary" disabled={!typed.includes("/") || !!cloning || !workspace} onClick={() => void clone(typed)}>
          {cloning ? "Cloning…" : "Clone"}
        </button>
      </div>
      {cloning && <div className="text-[12px] text-ink-2">Cloning {cloning} on the daemon machine. Large repositories can take a few minutes.</div>}
      <p className="text-[12px] text-ink-3">Uses <code className="font-mono">gh repo clone</code> with the GitHub CLI sign-in and Git protocol of the daemon machine (<code className="font-mono">git clone</code> over HTTPS without the GitHub CLI).</p>
      {q.isLoading && <div className="text-[12px] text-ink-3">Loading your repositories…</div>}
      {q.data && q.data.length === 0 && <div className="text-[12px] text-ink-3">No other repository found on GitHub yet. Type owner/name.</div>}
      {matches.length > 0 && (
        <div className="grid rounded-control border border-line bg-paper">
          {matches.slice(0, SHOWN).map((r) => (
            <div key={r.repo} className="flex h-9 items-center gap-2 border-b border-line-2 px-3 text-[13px] last:border-0">
              <span className="truncate font-mono text-[12px]">{r.repo}</span>
              {r.notifications > 0 && <span className="shrink-0 text-[11px] text-ink-3">{r.notifications} recent notification{r.notifications === 1 ? "" : "s"}</span>}
              <button className="btn btn-sm ml-auto" disabled={!!cloning || !workspace} onClick={() => void clone(r.repo)}>
                <Icon name="plus" size={12} />
                Clone
              </button>
            </div>
          ))}
          {matches.length > SHOWN && <div className="px-3 py-1.5 text-[12px] text-ink-3">{matches.length - SHOWN} more; type to filter.</div>}
        </div>
      )}
    </div>
  );
}
