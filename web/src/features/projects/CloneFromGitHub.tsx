// Add folder → Clone from GitHub: Home mirrors a repository and registers it as a Git folder; runners clone it from Home.
// Suggestions list your recently pushed repositories, read from GitHub when the dialog opens; any owner/name can be typed.
// An organization behind an IP allow list refuses Home's address: the error offers an exit route through a runner
// that can reach it (Settings → Home and runners → Exit routes), then clones again.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import type { CloneSuggestion } from "@/gen/CloneSuggestion";
import type { Project } from "@/gen/Project";
import { Icon } from "@/components/Icon";
import { toast, toastError } from "@/components/Toast";
import { api } from "@/lib/api";
import type { Workspace } from "@/gen/Workspace";
import { Choice, Choices, humanError } from "@/components/ui";
import { freeName, routeName, saveRoutes, useEgress, useExitRunners } from "@/lib/egress";
import { Link } from "@tanstack/react-router";

const SHOWN = 30;

/** `workspace`: the one workspace of the new folder, chosen in the Add folder dialog (null: not chosen yet). */
export function CloneFromGitHub({ workspace, onDone }: { workspace: Workspace | null; onDone?: () => void }) {
  const qc = useQueryClient();
  const [repo, setRepo] = useState("");
  const [cloning, setCloning] = useState<string | null>(null);
  // GitHub refused Home's address (IP allow list): the repository to clone through an exit route.
  const [offer, setOffer] = useState<string | null>(null);
  // POST reads your recently pushed repositories from GitHub (nothing is stored).
  const q = useQuery({ queryKey: ["clone-suggestions"], queryFn: () => api.post<CloneSuggestion[]>("/api/projects/clone/suggestions", {}) });
  const typed = repo.trim();
  const matches = (q.data ?? []).filter((r) => !typed || r.repo.toLowerCase().includes(typed.toLowerCase()));
  const clone = async (name: string) => {
    if (!workspace) return;
    setCloning(name);
    setOffer(null);
    try {
      const p = await api.post<Project>("/api/projects/clone", { repo: name, workspace_id: workspace.id });
      toast(`Added ${p.name} to ${workspace.name}`);
      onDone?.();
      setRepo("");
      await qc.invalidateQueries({ queryKey: ["projects"] });
      await qc.invalidateQueries({ queryKey: ["clone-suggestions"] });
    } catch (e) {
      if (/IP allow list/i.test(humanError(e))) setOffer(name.replace(/^https?:\/\/github\.com\//, ""));
      else toastError(e);
    } finally {
      setCloning(null);
    }
  };
  return (
    <div className="grid gap-2">
      <div className="text-[12px] text-ink-3">Home mirrors the repository{workspace ? ` and adds it to ${workspace.name}` : ". Choose its workspace below"}; runners clone it from Home on first use. A repository already cloned for another workspace gets a second folder.</div>
      <div className="flex gap-2">
        <input className="field-input flex-1 font-mono text-[12px]" aria-label="Repository" placeholder="owner/name, or filter the repositories below" value={repo} disabled={!!cloning} onChange={(e) => setRepo(e.target.value)} />
        <button className="btn btn-sm btn-primary" disabled={!typed.includes("/") || !!cloning || !workspace} onClick={() => void clone(typed)}>
          {cloning ? "Cloning…" : "Clone"}
        </button>
      </div>
      {cloning && <div className="text-[12px] text-ink-2">Cloning {cloning} on Home. Large repositories can take a few minutes.</div>}
      {offer && !cloning && <ExitOffer repo={offer} onAdded={() => void clone(offer)} onCancel={() => setOffer(null)} />}
      {q.isLoading && <div className="text-[12px] text-ink-3">Loading your repositories…</div>}
      {q.data && q.data.length === 0 && <div className="text-[12px] text-ink-3">No other repository found on GitHub yet. Type owner/name.</div>}
      {matches.length > 0 && (
        <div className="grid rounded-control border border-line bg-paper">
          {matches.slice(0, SHOWN).map((r) => (
            <div key={r.repo} className="flex min-h-9 items-center gap-2 border-b border-line-2 px-3 py-1 text-[13px] last:border-0">
              <span className="truncate font-mono text-[12px]">{r.repo}</span>
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

/** GitHub refused Home's address because the repository's organization has an IP allow list: send that
 * organization's traffic through a runner that can reach it (an exit route), then clone again. */
function ExitOffer({ repo, onAdded, onCancel }: { repo: string; onAdded: () => void; onCancel: () => void }) {
  const qc = useQueryClient();
  const org = repo.split("/")[0];
  const egress = useEgress();
  const runners = useExitRunners();
  const [via, setVia] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const chosen = via ?? runners[0]?.name ?? "";
  const add = async () => {
    const routes = egress.data?.routes ?? [];
    setBusy(true);
    setError(null);
    try {
      await saveRoutes(qc, [...routes, { name: freeName(routeName(org), routes), github_org: org, urls: [], via: chosen }]);
      toast(`Added route ${org} through ${chosen}`);
      onAdded();
    } catch (e) {
      setError(humanError(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="grid gap-2 rounded-card bg-attn-soft px-3 py-2.5 text-[13px]" role="alert">
      <span className="grid gap-0.5">
        <b className="text-attn">{org} accepts only allow-listed addresses</b>
        <span className="text-[12px] text-ink-2">GitHub refuses Home's address for {repo}. Send this organization's GitHub traffic through one of your runners that can reach it: an exit route.</span>
      </span>
      {runners.length === 0 ? (
        <span className="text-[12px] text-ink-2">Pair that runner first, in <Link to="/settings/runners" className="underline">Settings → Home and runners</Link>.</span>
      ) : (
        <span className="flex flex-wrap items-center gap-2">
          <span className="text-[12px] text-ink-2">Through</span>
          <Choices label="Exit runner">
            {runners.map((r) => (
              <Choice key={r.name} active={chosen === r.name} onClick={() => setVia(r.name)} title={r.online ? "Connected now" : "Not connected now"}>
                <span className={`size-1.5 rounded-full ${r.online ? "bg-gh-open" : "bg-ink-3"}`} aria-hidden="true" />
                {r.name}
              </Choice>
            ))}
          </Choices>
        </span>
      )}
      {error && <span className="text-[12px] text-danger-ink">{error}</span>}
      <span className="flex flex-wrap items-center gap-2">
        <button className="btn btn-sm btn-primary" disabled={!chosen || busy || egress.isLoading} onClick={() => void add()}>{busy ? "Adding…" : "Add route and clone"}</button>
        <button className="btn btn-sm btn-ghost" disabled={busy} onClick={onCancel}>Cancel</button>
      </span>
    </div>
  );
}
