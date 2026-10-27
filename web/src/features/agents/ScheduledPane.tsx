import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import type { ScheduledLaunch } from "@/gen/ScheduledLaunch";
import type { Session } from "@/gen/Session";
import type { Preview } from "@/gen/Preview";
import { useHarnessName } from "@/components/HarnessMark";
import { toastError } from "@/components/Toast";
import { api } from "@/lib/api";
import { useProjects, useSchedules, useSessions } from "@/lib/queries";
import { conditionsSummary, dependencyAgent } from "@/features/launch/DelaySelector";

export { useSchedules } from "@/lib/queries";

export function ScheduledPane({ launch, onStart, onClose }: { launch: ScheduledLaunch; onStart: (session: Session) => void; onClose: () => void }) {
  const harnessName = useHarnessName();
  const schedules = useSchedules().data ?? [];
  const sessions = useSessions().data ?? [];
  const projects = useProjects().data ?? [];
  const [busy, setBusy] = useState(false);
  const preview = useQuery({ queryKey: ["scheduled-preview", launch.id], queryFn: () => api.post<Preview>("/api/launch/preview", launch.spec), enabled: !launch.resume_session_id });
  const projectId = (launch.spec.target.kind === "project" || launch.spec.target.kind === "predraft") ? launch.spec.target.project_id : null;
  const name = projects.find((p) => p.id === projectId)?.name ?? (launch.spec.target.kind === "existing_predraft" ? "Local PR" : "New scratch folder");
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
        <b>{launch.resume_session_id ? "Follow-up · " : ""}{launch.status === "starting" ? "Starting…" : launch.status === "failed" ? "Launch needs attention" : "Scheduled"}</b>
        <p>{conditionsSummary(launch, sessions, schedules)}</p>
        {launch.conditions.flatMap((c) => c.kind === "agents" ? c.session_ids : []).length > 0 && <div className="grid gap-1">{launch.conditions.flatMap((c) => c.kind === "agents" ? c.session_ids : []).map((id) => { const s = dependencyAgent(id, sessions, schedules); return <div key={id} className="text-xs text-ink-2">{s?.title ?? "Missing or cancelled agent"} · {s?.status ?? "Unavailable"}</div>; })}</div>}
        <p className="text-xs text-ink-3">The daemon starts this work automatically when all conditions are met. It does not need an open window.</p>
        {launch.error && <p role="alert" className="text-gh-closed">{launch.error}</p>}
        <div className="flex gap-2">
          <button className="btn btn-primary" disabled={busy || launch.status === "starting"} onClick={() => void act(true)}>Start now</button>
          <button className="btn" disabled={busy || launch.status === "starting"} onClick={() => void act(false)}>{launch.resume_session_id ? "Cancel follow-up" : "Cancel launch"}</button>
        </div>
      </div>
      <p className="text-ink-2">{launch.spec.harness ? harnessName(launch.spec.harness) : "Agent"} · {name} · {launch.spec.model ?? "Default model"} · {launch.spec.effort ?? "Default effort"}</p>
      <div><b className="text-xs text-ink-2">{launch.resume_session_id ? "Message sent later (same session)" : "Prompt preview"}</b><pre className="mt-2 rounded-lg border border-line p-3 text-xs whitespace-pre-wrap">{launch.resume_session_id ? launch.spec.text : preview.data?.prompt ?? (preview.isError ? "Project context is unavailable. Cancel and schedule again with another project." : "Loading…")}</pre></div>
    </div>
  </div>;
}
