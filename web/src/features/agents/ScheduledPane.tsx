import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import type { ScheduledLaunch } from "@/gen/ScheduledLaunch";
import type { Session } from "@/gen/Session";
import type { Preview } from "@/gen/Preview";
import { HARNESS_NAME } from "@/components/HarnessMark";
import { toastError } from "@/components/Toast";
import { api } from "@/lib/api";
import { useProjects, useSessions } from "@/lib/queries";
import { delaySummary } from "@/features/launch/DelaySelector";

export const useSchedules = () => useQuery({ queryKey: ["schedules"], queryFn: () => api.get<ScheduledLaunch[]>("/api/launch/schedules"), staleTime: Infinity });

export function ScheduledPane({ launch, onStart, onClose }: { launch: ScheduledLaunch; onStart: (session: Session) => void; onClose: () => void }) {
  const sessions = useSessions().data ?? [];
  const projects = useProjects().data ?? [];
  const [busy, setBusy] = useState(false);
  const preview = useQuery({ queryKey: ["scheduled-preview", launch.id], queryFn: () => api.post<Preview>("/api/launch/preview", launch.spec) });
  const projectId = launch.spec.target.kind === "project" ? launch.spec.target.project_id : null;
  const name = projects.find((p) => p.id === projectId)?.name ?? "New temporary folder";
  const act = async (start: boolean) => {
    setBusy(true);
    try {
      if (start) onStart(await api.post<Session>(`/api/launch/schedules/${launch.id}/start`));
      else { await api.delete(`/api/launch/schedules/${launch.id}`); onClose(); }
    } catch (e) { toastError(e); }
    finally { setBusy(false); }
  };
  return <div className="flex h-full flex-col bg-paper">
    <div className="flex items-center gap-2 border-b border-line px-4 py-3">
      <b className="min-w-0 flex-1 truncate text-sm">{launch.title}</b>
      <button className="btn btn-sm" onClick={onClose}>Close panel</button>
    </div>
    <div className="grid gap-4 overflow-y-auto p-4 text-sm">
      <div className="grid gap-2 rounded-lg border border-line bg-chrome p-3">
        <b>{launch.status === "starting" ? "Starting…" : launch.status === "failed" ? "Launch needs attention" : "Scheduled"}</b>
        <p>{delaySummary(launch.delay, launch.ready_at, sessions)}</p>
        {launch.delay.kind === "agents" && <div className="grid gap-1">{launch.delay.session_ids.map((id) => { const s = sessions.find((s) => s.id === id); return <div key={id} className="text-xs text-ink-2">{s?.title ?? "Missing agent"} · {s?.status === "waiting" ? "Done" : s?.status === "closed" ? "Archived" : s?.status ?? "Unavailable"}</div>; })}</div>}
        <p className="text-xs text-ink-3">The daemon starts this agent automatically when the condition is met. It does not need an open window.</p>
        {launch.error && <p role="alert" className="text-gh-closed">{launch.error}</p>}
        <div className="flex gap-2">
          <button className="btn btn-primary" disabled={busy || launch.status === "starting"} onClick={() => void act(true)}>Start now</button>
          <button className="btn" disabled={busy || launch.status === "starting"} onClick={() => void act(false)}>Cancel launch</button>
        </div>
      </div>
      <p className="text-ink-2">{launch.spec.harness ? HARNESS_NAME[launch.spec.harness] : "Agent"} · {name} · {launch.spec.model ?? "Default model"} · {launch.spec.effort ?? "Default effort"}</p>
      <div><b className="text-xs text-ink-2">Prompt preview</b><pre className="mt-2 rounded-lg border border-line p-3 text-xs whitespace-pre-wrap">{preview.data?.prompt ?? (preview.isError ? "Project context is unavailable. Cancel and schedule again with another project." : "Loading…")}</pre></div>
    </div>
  </div>;
}
