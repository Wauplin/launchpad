// Projects → Clone from GitHub: the daemon clones a repository into the projects root and registers it.
// Suggestions come from repositories already in the GitHub cache; any owner/name can be typed.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import type { Project } from "@/gen/Project";
import { Icon } from "@/components/Icon";
import { toast, toastError } from "@/components/Toast";
import { api } from "@/lib/api";
import { useScope } from "@/lib/workspace";

const SHOWN = 30;

export function CloneFromGitHub({ root, host = null, onClose }: { root: string | undefined; host?: string | null; onClose: () => void }) {
  const qc = useQueryClient();
  const workspaceId = useScope().workspace?.id ?? null;
  const [repo, setRepo] = useState("");
  const [cloning, setCloning] = useState<string | null>(null);
  const q = useQuery({ queryKey: ["clone-suggestions"], queryFn: () => api.get<string[]>("/api/projects/clone") });
  const typed = repo.trim();
  const matches = (q.data ?? []).filter((r) => !typed || r.toLowerCase().includes(typed.toLowerCase()));
  const clone = async (name: string) => {
    setCloning(name);
    try {
      const p = await api.post<Project>("/api/projects/clone", { repo: name, host, workspace_id: workspaceId });
      toast(`Cloned ${name} into ${p.path}`);
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
    <div className="grid gap-2 rounded-lg border border-line bg-chrome p-3">
      <div className="flex items-center gap-2">
        <b className="text-[12.5px]">Clone from GitHub</b>
        <span className="min-w-0 truncate text-xs text-ink-3">into {root ?? "the projects root"}, then added as a project</span>
        <button className="iconbtn ml-auto" title="Close" onClick={onClose}>
          <Icon name="x" size={14} />
        </button>
      </div>
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
            <div key={r} className="flex h-9 items-center gap-2 border-b border-line-2 px-3 text-[12.5px] last:border-0">
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
