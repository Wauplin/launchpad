import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import type { CronAgent } from "@/gen/CronAgent";
import type { Session } from "@/gen/Session";
import type { Preview } from "@/gen/Preview";
import { HARNESS_NAME } from "@/components/HarnessMark";
import { toastError } from "@/components/Toast";
import { api } from "@/lib/api";
import { useProjects, useSessions } from "@/lib/queries";
import { isDone } from "@/lib/sessionStatus";
import { CronFields, conditionSummary, type CronDraft } from "@/features/launch/CronSelector";

export function cronSummary(cron: CronAgent): string {
  if (cron.paused) return `Paused · ${cron.schedule}`;
  return `${cron.schedule}${cron.next_run_at ? ` · next ${new Date(cron.next_run_at).toLocaleString()}` : ""}`;
}

export function CronPane({ cron, onOpen, onClose }: { cron: CronAgent; onOpen: (session: Session) => void; onClose: () => void }) {
  const sessions = useSessions().data ?? [];
  const projects = useProjects().data ?? [];
  const [busy, setBusy] = useState(false);
  const [edit, setEdit] = useState<CronDraft | null>(null);
  const [editValid, setEditValid] = useState(false);
  const preview = useQuery({ queryKey: ["cron-preview", cron.id], queryFn: () => api.post<Preview>("/api/launch/preview", cron.spec) });
  const projectId = (cron.spec.target.kind === "project" || cron.spec.target.kind === "predraft") ? cron.spec.target.project_id : null;
  const name = projects.find((p) => p.id === projectId)?.name ?? (cron.spec.target.kind === "existing_predraft" ? "Local pre-draft" : "New temporary folder");
  const runs = cron.runs.slice().reverse().map((id) => ({ id, session: sessions.find((s) => s.id === id) }));
  const act = async (f: () => Promise<void>) => {
    setBusy(true);
    try { await f(); } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  return <div className="flex h-full flex-col bg-paper">
    <div className="flex items-center gap-2 border-b border-line px-4 py-3">
      <b className="min-w-0 flex-1 truncate text-sm">{cron.title}</b>
      <button className="btn btn-sm" onClick={onClose}>Close panel</button>
    </div>
    <div className="grid gap-4 overflow-y-auto p-4 text-sm">
      <div className="grid gap-2 rounded-lg border border-line bg-chrome p-3">
        <b>{cron.paused ? "Cron paused" : "Cron active"}</b>
        {edit ? <>
          <CronFields draft={edit} harness={cron.spec.harness} onChange={setEdit} onValidityChange={setEditValid} />
          <div className="flex gap-2">
            <button className="btn btn-primary" disabled={busy || !editValid} onClick={() => void act(async () => { await api.patch(`/api/launch/crons/${cron.id}`, edit); setEdit(null); })}>Save</button>
            <button className="btn" disabled={busy} onClick={() => setEdit(null)}>Cancel</button>
          </div>
        </> : <>
          <p><span className="font-mono">{cron.schedule}</span> · {cron.paused ? "No run is planned." : cron.next_run_at ? `Next run ${new Date(cron.next_run_at).toLocaleString()}` : ""}</p>
          {cron.conditions.length ? <ul className="list-disc pl-5 text-xs text-ink-2">{cron.conditions.map((c, i) => <li key={i}>{conditionSummary(c, cron.spec.harness)}</li>)}</ul> : <p className="text-xs text-ink-2">No conditions: every scheduled time starts a run.</p>}
          {cron.last_outcome && <p className={`text-xs ${cron.last_outcome.startsWith("Started") ? "text-ink-2" : "text-gh-closed"}`}>Last check{cron.last_check_at ? ` ${new Date(cron.last_check_at).toLocaleString()}` : ""}: {cron.last_outcome}</p>}
          <p className="text-xs text-ink-3">The daemon starts each run automatically. It does not need an open window. Run now ignores the conditions and keeps the schedule.</p>
          <div className="flex flex-wrap gap-2">
            <button className="btn btn-primary" disabled={busy} onClick={() => void act(async () => onOpen(await api.post<Session>(`/api/launch/crons/${cron.id}/run`)))}>Run now</button>
            <button className="btn" disabled={busy} onClick={() => setEdit({ schedule: cron.schedule, conditions: cron.conditions })}>Edit schedule</button>
            <button className="btn" disabled={busy} onClick={() => void act(async () => { await api.patch(`/api/launch/crons/${cron.id}`, { paused: !cron.paused }); })}>{cron.paused ? "Resume" : "Pause"}</button>
            <button className="btn" disabled={busy} onClick={() => void act(async () => { await api.delete(`/api/launch/crons/${cron.id}`); onClose(); })}>Delete cron</button>
          </div>
        </>}
      </div>
      <p className="text-ink-2">{cron.spec.harness ? HARNESS_NAME[cron.spec.harness] : "Agent"} · {name} · {cron.spec.model ?? "Default model"} · {cron.spec.effort ?? "Default effort"}</p>
      <div className="grid gap-1">
        <b className="text-xs text-ink-2">Runs ({cron.runs.length})</b>
        {runs.length ? runs.map(({ id, session }) => <button key={id} disabled={!session} onClick={() => session && onOpen(session)} className="flex items-center gap-2 rounded-md border border-line px-2 py-1.5 text-left text-xs hover:bg-chrome disabled:opacity-60">
          <span className="min-w-0 flex-1 truncate">{session ? new Date(session.started_at).toLocaleString() : "Deleted session"}</span>
          <span className="text-ink-3">{session ? (isDone(session) ? "Done" : session.status) : ""}</span>
        </button>) : <p className="text-xs text-ink-3">No run yet.</p>}
      </div>
      <div><b className="text-xs text-ink-2">Prompt preview</b><pre className="mt-2 rounded-lg border border-line p-3 text-xs whitespace-pre-wrap">{preview.data?.prompt ?? (preview.isError ? "Project context is unavailable. Delete this cron and create it again with another project." : "Loading…")}</pre></div>
    </div>
  </div>;
}
