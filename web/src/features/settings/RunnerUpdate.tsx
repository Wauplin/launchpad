// One-click runner update: a runner with another build than Home's downloads Home's build through its tunnel and
// restarts itself once none of its agents is in a turn (launchpad_core::self_update). Clicked on Home (any outdated
// runner) or on the runner itself.

import { useQuery } from "@tanstack/react-query";
import type { RunnerStatus } from "@/gen/RunnerStatus";
import type { RunnerUpdate } from "@/gen/RunnerUpdate";
import type { Status } from "@/gen/Status";
import { ConfirmButton } from "@/components/ui";
import { toastError } from "@/components/Toast";
import { api } from "@/lib/api";
import { useStatus } from "@/lib/queries";

export function updateLabel(u: RunnerUpdate): string {
  switch (u.state) {
    case "waiting": return u.running === 0 ? "Starting the update…" : `Updates when ${u.running === 1 ? "its agent finishes its turn" : `its ${u.running} agents finish their turn`}`;
    case "downloading": return "Downloading Home's build…";
    case "restarting": return "Restarting with Home's build…";
    case "failed": return `Update failed: ${u.error}`;
  }
}

function UpdateControls({ name, update, start, cancel }: { name: string; update: RunnerUpdate | null; start: () => unknown; cancel: () => unknown }) {
  if (update && update.state !== "failed") {
    return (
      <span className="inline-flex flex-wrap items-center gap-2">
        <span className="font-semibold text-teal-ink">{updateLabel(update)}</span>
        {update.state === "waiting" && <button className="btn btn-sm" onClick={() => void cancel()}>Cancel</button>}
      </span>
    );
  }
  return (
    <span className="inline-flex flex-wrap items-center gap-2">
      <ConfirmButton tone="publish" icon="download" confirmLabel={`Update ${name}`} consequence={`Installs Home's build on ${name} and restarts it as soon as none of its agents is in a turn. Its open terminals close; its sessions continue after the restart.`} onConfirm={start}>Update {name}</ConfirmButton>
      {update?.state === "failed" && <span className="text-gh-closed">{updateLabel(update)}</span>}
    </span>
  );
}

/** On Home: the Update button of an outdated runner, with the progress read from the runner's status. */
export function UpdateRunnerButton({ runner }: { runner: RunnerStatus }) {
  const id = runner.runner.id;
  const q = useQuery({ queryKey: ["runner-status", id], queryFn: () => api.get<Status>(`/api/runners/${id}/status`), enabled: runner.connected, refetchInterval: 4000, retry: false });
  const update = q.data?.runner_update ?? null;
  return (
    <UpdateControls
      name={runner.runner.name}
      update={update}
      start={() => api.post(`/api/runners/${id}/update`, {}).then(() => q.refetch(), toastError)}
      cancel={() => api.delete(`/api/runners/${id}/update`).then(() => q.refetch(), toastError)}
    />
  );
}

/** On a runner: update this runner from Home. */
export function UpdateThisRunnerButton() {
  const update = useStatus().data?.runner_update ?? null;
  return <UpdateControls name="this runner" update={update} start={() => api.post("/api/runner/update", {}).catch(toastError)} cancel={() => api.delete("/api/runner/update").catch(toastError)} />;
}

/** A connected outdated runner whose build can update itself on Home's request. */
export function canSelfUpdate(r: RunnerStatus): boolean {
  return r.connected && r.compat === "outdated" && r.runner.capabilities.self_update;
}
