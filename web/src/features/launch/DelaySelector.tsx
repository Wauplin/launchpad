import { memo, useEffect, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { LaunchDelay } from "@/gen/LaunchDelay";
import type { Session } from "@/gen/Session";
import type { HarnessKind } from "@/gen/HarnessKind";
import { useStatus } from "@/app/StatusBar";
import { HARNESS_NAME, HarnessMark } from "@/components/HarnessMark";
import { useSessions } from "@/lib/queries";
import { isDone } from "@/lib/sessionStatus";

export function delaySummary(delay: LaunchDelay, readyAt: number | null = null, sessions: Session[] = []): string {
  if (delay.kind === "agents") return `When ${delay.session_ids.map((id) => sessions.find((s) => s.id === id)?.title ?? "missing agent").join(" and ")} ${delay.session_ids.length === 1 ? "is" : "are"} Done`;
  const at = readyAt ?? (delay.kind === "at" ? delay.at : null);
  if (delay.kind === "quota") return `${HARNESS_NAME[delay.harness]} ${delay.window === "five_hour" ? "5h" : "weekly"} reset${at ? ` · ${new Date(at).toLocaleString()}` : ""}`;
  return at ? new Date(at).toLocaleString() : delay.kind === "in" ? `In ${delay.minutes} minutes` : "Pick a date and time";
}

export function delayError(delay: LaunchDelay | null): string | null {
  if (!delay) return null;
  if (delay.kind === "in" && (!Number.isInteger(delay.minutes) || delay.minutes < 1 || delay.minutes > 525600)) return "Enter a delay from 1 minute to 1 year.";
  if (delay.kind === "at" && (!Number.isFinite(delay.at) || delay.at <= Date.now() || delay.at > Date.now() + 366 * 86400000)) return "Choose a future date within the next year.";
  if (delay.kind === "agents" && !delay.session_ids.length) return "Select the agents to wait for.";
  return null;
}

function localDate(at: number) {
  const d = new Date(at);
  if (!Number.isFinite(at) || at <= 0) return "";
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}T${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

const AgentChoice = memo(function AgentChoice({ session, checked, toggle }: { session: Session; checked: boolean; toggle: () => void }) {
  return <label className="flex h-12 cursor-pointer items-center gap-2 border-b border-line px-2 text-xs hover:bg-chrome">
    <input type="checkbox" className="accent-teal" checked={checked} onChange={toggle} />
    <HarnessMark harness={session.harness} size={14} />
    <span className="min-w-0 flex-1 truncate" title={session.title}>{session.title}</span>
    <span className="text-ink-3">{isDone(session) ? "Done" : session.status === "closed" ? "Archived" : session.status}</span>
  </label>;
});

function AgentPicker({ ids, onChange }: { ids: string[]; onChange: (ids: string[]) => void }) {
  const all = useSessions();
  const [filter, setFilter] = useState("");
  const ref = useRef<HTMLDivElement>(null);
  const sessions = (all.data ?? []).filter((s) => (s.status !== "closed" || ids.includes(s.id)) && s.title.toLowerCase().includes(filter.toLowerCase()));
  const virtual = useVirtualizer({ count: sessions.length, getScrollElement: () => ref.current, estimateSize: () => 48, overscan: 3, useFlushSync: false });
  return <div className="grid gap-1.5">
    <input className="field-input" aria-label="Find agents to wait for" placeholder="Find an agent…" value={filter} onChange={(e) => setFilter(e.target.value)} />
    <div ref={ref} className="max-h-48 overflow-y-auto rounded-md border border-line">
      <div style={{ height: virtual.getTotalSize(), position: "relative" }}>
        {virtual.getVirtualItems().map((row) => { const s = sessions[row.index]; return <div key={s.id} style={{ position: "absolute", top: 0, left: 0, width: "100%", height: row.size, transform: `translateY(${row.start}px)` }}>
          <AgentChoice session={s} checked={ids.includes(s.id)} toggle={() => onChange(ids.includes(s.id) ? ids.filter((id) => id !== s.id) : [...ids, s.id])} />
        </div>; })}
      </div>
      {!sessions.length && <div className="p-2 text-xs text-ink-3">{all.isLoading ? "Loading agents…" : "No matching agents."}</div>}
    </div>
    <p className="text-xs text-ink-3">{ids.length} selected · Starts when all selected agents are Done (waiting for input). Archived or interrupted agents do not count as Done.</p>
  </div>;
}

export function DelaySelector({ delay, harness, onChange, onValidityChange }: { delay: LaunchDelay | null; harness: HarnessKind | null; onChange: (delay: LaunchDelay | null) => void; onValidityChange: (valid: boolean) => void }) {
  const status = useStatus().data;
  const [, tick] = useState(0);
  useEffect(() => { const timer = setInterval(() => tick((n) => n + 1), 1000); return () => clearInterval(timer); }, []);
  let error = delayError(delay);
  let reset: number | null = null;
  if (delay?.kind === "quota") {
    const h = status?.[delay.harness];
    const q = h?.[delay.window];
    reset = q ? q.resets_at * 1000 : null;
    if (!h || h.quota_error || !h.quota_at || Date.now() - h.quota_at > 15 * 60000 || !reset || reset <= Date.now()) error = "Quota reset is unavailable. Wait for a fresh quota reading, or choose a time.";
  }
  const valid = !error;
  useEffect(() => { onValidityChange(valid); }, [valid, onValidityChange]);
  return <div className="grid gap-1.5">
    <label className="text-[11.5px] font-[650] text-ink-2" htmlFor="launch-start">Start</label>
    <select id="launch-start" className="field-input h-8" value={delay?.kind ?? "now"} onChange={(e) => {
      switch (e.target.value) {
        case "in": onChange({ kind: "in", minutes: 30 }); break;
        case "at": { const d = new Date(); d.setHours(7, 0, 0, 0); if (d.getTime() <= Date.now()) d.setDate(d.getDate() + 1); onChange({ kind: "at", at: d.getTime() }); break; }
        case "agents": onChange({ kind: "agents", session_ids: [] }); break;
        case "quota": onChange({ kind: "quota", harness: harness ?? "codex", window: "five_hour" }); break;
        default: onChange(null);
      }
    }}>
      <option value="now">Now</option><option value="in">In…</option><option value="at">At a date and time…</option><option value="agents">After agents are Done…</option><option value="quota">When a quota resets…</option>
    </select>
    {delay && <div className="grid gap-2 rounded-lg border border-line bg-chrome p-2.5">
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
      <p className="text-xs text-ink-3">Queued locally. Launchpad starts it automatically while the daemon is running, including after a restart.</p>
    </div>}
  </div>;
}
