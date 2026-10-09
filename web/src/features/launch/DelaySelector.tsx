import { memo, useEffect, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { LaunchDelay } from "@/gen/LaunchDelay";
import type { ScheduledLaunch } from "@/gen/ScheduledLaunch";
import type { Session } from "@/gen/Session";
import type { HarnessKind } from "@/gen/HarnessKind";
import { useStatus } from "@/app/StatusBar";
import { HARNESS_NAME, HarnessMark } from "@/components/HarnessMark";
import { useSchedules, useSessions } from "@/lib/queries";
import { isDone } from "@/lib/sessionStatus";

type DependencyAgent = { id: string; title: string; harness: HarnessKind | null; status: string };

function sessionDependency(session: Session): DependencyAgent {
  return { id: session.id, title: session.title, harness: session.harness, status: isDone(session) ? "Done" : session.status === "closed" ? "Archived" : session.status };
}

function scheduledDependency(launch: ScheduledLaunch): DependencyAgent {
  return { id: launch.id, title: launch.title, harness: launch.spec.harness, status: launch.status === "queued" ? "Scheduled" : launch.status === "failed" ? "Needs attention" : "Starting" };
}

export function dependencyAgent(id: string, sessions: Session[], schedules: ScheduledLaunch[]): DependencyAgent | undefined {
  const session = sessions.find((s) => s.id === id);
  if (session) return sessionDependency(session);
  const launch = schedules.find((s) => s.id === id);
  if (launch) return scheduledDependency(launch);
}

function conditionLabel(delay: LaunchDelay, sessions: Session[], schedules: ScheduledLaunch[]): string | null {
  if (delay.kind === "agents") return `When ${delay.session_ids.map((id) => dependencyAgent(id, sessions, schedules)?.title ?? "missing or cancelled agent").join(" and ")} ${delay.session_ids.length === 1 ? "is" : "are"} Done`;
  if (delay.kind === "quota") return `${HARNESS_NAME[delay.harness]} ${delay.window === "five_hour" ? "5h" : "weekly"} reset`;
  return null;
}

/** One line for all conditions of a scheduled launch; time and quota conditions share the resolved `ready_at`. */
export function conditionsSummary(launch: ScheduledLaunch, sessions: Session[] = [], schedules: ScheduledLaunch[] = []): string {
  const parts = launch.conditions.map((c) => conditionLabel(c, sessions, schedules)).filter((p): p is string => !!p);
  if (launch.ready_at) parts.push(`${parts.length ? "not before " : ""}${new Date(launch.ready_at).toLocaleString()}`);
  return parts.join(" · ");
}

function conditionError(delay: LaunchDelay): string | null {
  if (delay.kind === "in" && (!Number.isInteger(delay.minutes) || delay.minutes < 1 || delay.minutes > 525600)) return "Enter a delay from 1 minute to 1 year.";
  if (delay.kind === "at" && (!Number.isFinite(delay.at) || delay.at <= Date.now() || delay.at > Date.now() + 366 * 86400000)) return "Choose a future date within the next year.";
  if (delay.kind === "agents" && !delay.session_ids.length) return "Select the agents to wait for.";
  return null;
}

export function conditionsError(conditions: LaunchDelay[]): string | null {
  for (const c of conditions) { const error = conditionError(c); if (error) return error; }
  return null;
}

const KINDS: { kind: LaunchDelay["kind"]; label: string }[] = [
  { kind: "in", label: "After a delay" },
  { kind: "at", label: "At a date and time" },
  { kind: "agents", label: "After agents are Done" },
  { kind: "quota", label: "When a quota resets" },
];

function newCondition(kind: string, harness: HarnessKind | null): LaunchDelay | null {
  switch (kind) {
    case "in": return { kind: "in", minutes: 30 };
    case "at": { const d = new Date(); d.setHours(7, 0, 0, 0); if (d.getTime() <= Date.now()) d.setDate(d.getDate() + 1); return { kind: "at", at: d.getTime() }; }
    case "agents": return { kind: "agents", session_ids: [] };
    case "quota": return { kind: "quota", harness: harness ?? "codex", window: "five_hour" };
    default: return null;
  }
}

function localDate(at: number) {
  const d = new Date(at);
  if (!Number.isFinite(at) || at <= 0) return "";
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}T${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

const AgentChoice = memo(function AgentChoice({ session, checked, toggle }: { session: DependencyAgent; checked: boolean; toggle: () => void }) {
  return <label className="flex h-12 cursor-pointer items-center gap-2 border-b border-line px-2 text-xs hover:bg-chrome">
    <input type="checkbox" className="accent-teal" checked={checked} onChange={toggle} />
    {session.harness && <HarnessMark harness={session.harness} size={14} />}
    <span className="min-w-0 flex-1 truncate" title={session.title}>{session.title}</span>
    <span className="text-ink-3">{session.status}</span>
  </label>;
});

function AgentPicker({ ids, onChange }: { ids: string[]; onChange: (ids: string[]) => void }) {
  const all = useSessions();
  const scheduled = useSchedules();
  const [filter, setFilter] = useState("");
  const ref = useRef<HTMLDivElement>(null);
  const existing = all.data ?? [];
  const queue = scheduled.data ?? [];
  const sessionIds = new Set(existing.map((s) => s.id));
  const agents = [
    ...queue.filter((s) => !s.resume_session_id && !sessionIds.has(s.id)).map(scheduledDependency),
    ...existing.filter((s) => s.status !== "closed" || ids.includes(s.id)).map(sessionDependency),
  ];
  const sessions = agents.filter((s) => s.title.toLowerCase().includes(filter.toLowerCase()));
  const virtual = useVirtualizer({ count: sessions.length, getScrollElement: () => ref.current, estimateSize: () => 48, overscan: 3, useFlushSync: false });
  return <div className="grid gap-1.5">
    <input className="field-input" aria-label="Find agents to wait for" placeholder="Find an agent…" value={filter} onChange={(e) => setFilter(e.target.value)} />
    <div ref={ref} className="max-h-48 overflow-y-auto rounded-md border border-line">
      <div style={{ height: virtual.getTotalSize(), position: "relative" }}>
        {virtual.getVirtualItems().map((row) => { const s = sessions[row.index]; return <div key={s.id} style={{ position: "absolute", top: 0, left: 0, width: "100%", height: row.size, transform: `translateY(${row.start}px)` }}>
          <AgentChoice session={s} checked={ids.includes(s.id)} toggle={() => onChange(ids.includes(s.id) ? ids.filter((id) => id !== s.id) : [...ids, s.id])} />
        </div>; })}
      </div>
      {!sessions.length && <div className="p-2 text-xs text-ink-3">{all.isLoading || scheduled.isLoading ? "Loading agents…" : "No matching agents."}</div>}
    </div>
    <p className="text-xs text-ink-3">{ids.length} selected · Starts when all selected agents are Done (waiting for input). Scheduled agents must launch and reach Done first. Cancelled, archived or interrupted agents do not count as Done.</p>
  </div>;
}

function quotaReset(delay: LaunchDelay, status: ReturnType<typeof useStatus>["data"]): { reset: number | null; error: string | null } {
  if (delay.kind !== "quota") return { reset: null, error: null };
  const h = status?.[delay.harness];
  const q = h?.[delay.window];
  const reset = q ? q.resets_at * 1000 : null;
  if (!h || h.quota_error || !h.quota_at || Date.now() - h.quota_at > 15 * 60000 || !reset || reset <= Date.now()) return { reset, error: "Quota reset is unavailable. Wait for a fresh quota reading, or choose a time." };
  return { reset, error: null };
}

function ConditionEditor({ delay, onChange, onRemove }: { delay: LaunchDelay; onChange: (delay: LaunchDelay) => void; onRemove: () => void }) {
  const quota = quotaReset(delay, useStatus().data);
  const error = conditionError(delay) ?? quota.error;
  const reset = quota.reset;
  return <div className="grid gap-2 rounded-lg border border-line bg-chrome p-2.5">
    <div className="flex items-center gap-2">
      <b className="flex-1 text-xs text-ink-2">{KINDS.find((k) => k.kind === delay.kind)?.label}</b>
      <button className="btn btn-sm btn-ghost" onClick={onRemove}>Remove</button>
    </div>
    {delay.kind === "in" && <>
      <div className="flex gap-1.5">{[15, 30, 60].map((minutes) => <button key={minutes} className={`btn btn-sm ${delay.minutes === minutes ? "btn-primary" : ""}`} onClick={() => onChange({ kind: "in", minutes })}>{minutes === 60 ? "1 hour" : `${minutes} min`}</button>)}</div>
      <label className="flex items-center gap-2 text-xs"><input aria-label="Delay in minutes" className="field-input w-24" type="number" min={1} max={525600} step={1} value={delay.minutes || ""} onChange={(e) => onChange({ kind: "in", minutes: Number(e.target.value) })} />minutes after you click Schedule</label>
    </>}
    {delay.kind === "at" && <><input aria-label="Start date and time" type="datetime-local" className="field-input" value={localDate(delay.at)} onChange={(e) => onChange({ kind: "at", at: e.target.value ? new Date(e.target.value).getTime() : 0 })} /><p className="text-xs text-ink-3">Your local time · {Intl.DateTimeFormat().resolvedOptions().timeZone}</p></>}
    {delay.kind === "agents" && <AgentPicker ids={delay.session_ids} onChange={(session_ids) => onChange({ kind: "agents", session_ids })} />}
    {delay.kind === "quota" && <>
      <div className="grid grid-cols-2 gap-2">
        <select aria-label="Quota provider" className="field-input" value={delay.harness} onChange={(e) => onChange({ ...delay, harness: e.target.value as HarnessKind })}><option value="codex">Codex</option><option value="claude">Claude Code</option></select>
        <select aria-label="Quota window" className="field-input" value={delay.window} onChange={(e) => onChange({ ...delay, window: e.target.value as "five_hour" | "weekly" })}><option value="five_hour">5-hour quota</option><option value="weekly">Weekly quota</option></select>
      </div>
      {reset && !error && <p className="text-xs text-ink-2">Resets {new Date(reset).toLocaleString()}. This reset time is saved when you schedule.</p>}
    </>}
    {error && <p role="alert" className="text-xs text-gh-closed">{error}</p>}
  </div>;
}

export function DelaySelector({ conditions, harness, onChange, onValidityChange }: { conditions: LaunchDelay[]; harness: HarnessKind | null; onChange: (conditions: LaunchDelay[]) => void; onValidityChange: (valid: boolean) => void }) {
  const status = useStatus().data;
  const [, tick] = useState(0);
  useEffect(() => { const timer = setInterval(() => tick((n) => n + 1), 1000); return () => clearInterval(timer); }, []);
  const valid = !conditionsError(conditions) && conditions.every((c) => !quotaReset(c, status).error);
  useEffect(() => { onValidityChange(valid); }, [valid, onValidityChange]);
  const hasAgents = conditions.some((c) => c.kind === "agents");
  return <div className="grid gap-1.5">
    <label className="text-[11.5px] font-[650] text-ink-2" htmlFor="launch-start">Start</label>
    <p className="text-xs text-ink-3">{!conditions.length ? "Now, when you click Launch. Add conditions to schedule it." : conditions.length === 1 ? "When this condition is met." : "When all of these conditions are met."}</p>
    {conditions.map((c, i) => <ConditionEditor key={i} delay={c} onChange={(next) => onChange(conditions.map((x, j) => j === i ? next : x))} onRemove={() => onChange(conditions.filter((_, j) => j !== i))} />)}
    <select id="launch-start" className="field-input h-8" value="" onChange={(e) => { const c = newCondition(e.target.value, harness); if (c) onChange([...conditions, c]); }}>
      <option value="">{conditions.length ? "Add another condition…" : "Add a start condition…"}</option>
      {KINDS.map((k) => <option key={k.kind} value={k.kind} disabled={k.kind === "agents" && hasAgents}>{k.label}…</option>)}
    </select>
    {!!conditions.length && <p className="text-xs text-ink-3">Queued locally. Launchpad starts it automatically while the daemon is running, including after a restart.</p>}
  </div>;
}
