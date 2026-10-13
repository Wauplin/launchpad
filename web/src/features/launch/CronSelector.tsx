import { useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import type { CronCondition } from "@/gen/CronCondition";
import type { HarnessKind } from "@/gen/HarnessKind";
import { useHarnessName, useHarnessInfo } from "@/components/HarnessMark";
import { api } from "@/lib/api";
import { useServerTimeZone } from "@/lib/serverTime";

export type CronDraft = { schedule: string; conditions: CronCondition[] };

export const DEFAULT_CRON: CronDraft = { schedule: "0 9 * * 1-5", conditions: [{ kind: "previous_run_finished" }, { kind: "quota_left", window: "five_hour", min_left_percent: 20 }] };

const PRESETS: [string, string][] = [["Every hour", "0 * * * *"], ["Every day at 9:00", "0 9 * * *"], ["Weekdays at 9:00", "0 9 * * 1-5"], ["Mondays at 9:00", "0 9 * * 1"]];

export function conditionSummary(c: CronCondition, harness: HarnessKind | null, name: (id: HarnessKind) => string = (id) => id): string {
  if (c.kind === "previous_run_finished") return "Previous run is no longer Running";
  return `${harness ? name(harness) : "Agent"} has at least ${c.min_left_percent}% of its ${c.window === "five_hour" ? "5-hour" : "weekly"} quota left`;
}

/** Next run times for a schedule, checked by the daemon (same parser as the scheduler). */
export function useCronNext(schedule: string) {
  const [debounced, setDebounced] = useState(schedule);
  useEffect(() => { const t = setTimeout(() => setDebounced(schedule.trim()), 300); return () => clearTimeout(t); }, [schedule]);
  return useQuery({ queryKey: ["cron-next", debounced], queryFn: () => api.get<number[]>(`/api/launch/crons/next?schedule=${encodeURIComponent(debounced)}`), enabled: !!debounced, retry: false });
}

/** Schedule field and condition list, shared by the composer and the cron pane. */
export function CronFields({ draft, harness, onChange, onValidityChange }: { draft: CronDraft; harness: HarnessKind | null; onChange: (draft: CronDraft) => void; onValidityChange: (valid: boolean) => void }) {
  const name = useHarnessName();
  const supportsQuota = useHarnessInfo(harness)?.capabilities.quota ?? false;
  const next = useCronNext(draft.schedule);
  const tz = useServerTimeZone();
  const browserZone = Intl.DateTimeFormat().resolvedOptions().timeZone;
  const previous = draft.conditions.some((c) => c.kind === "previous_run_finished");
  const quotas = draft.conditions.filter((c) => c.kind === "quota_left");
  const badPercent = quotas.some((c) => !Number.isInteger(c.min_left_percent) || c.min_left_percent < 1 || c.min_left_percent > 100);
  const valid = !!draft.schedule.trim() && next.isSuccess && !next.isFetching && !badPercent && (!quotas.length || supportsQuota);
  useEffect(() => { onValidityChange(valid); }, [valid, onValidityChange]);
  const setQuota = (i: number, q: Extract<CronCondition, { kind: "quota_left" }> | null) => {
    const rest = quotas.filter((_, j) => j !== i);
    if (q) rest.splice(i, 0, q);
    onChange({ ...draft, conditions: [...(previous ? [{ kind: "previous_run_finished" as const }] : []), ...rest] });
  };
  return <div className="grid gap-2">
    <div className="flex flex-wrap gap-1.5">{PRESETS.map(([label, schedule]) => <button key={schedule} className={`btn btn-sm ${draft.schedule.trim() === schedule ? "btn-primary" : ""}`} onClick={() => onChange({ ...draft, schedule })}>{label}</button>)}</div>
    <input aria-label="Cron schedule" className="field-input font-mono" placeholder="minute hour day month weekday" value={draft.schedule} onChange={(e) => onChange({ ...draft, schedule: e.target.value })} />
    {next.error ? <p role="alert" className="text-xs text-gh-closed">{next.error.message}</p>
      : next.data && <p className="text-xs text-ink-2">Next runs{tz?.differs ? ` (your time, ${browserZone})` : ""}: {next.data.map((at) => new Date(at).toLocaleString()).join(" · ")}</p>}
    <p className="text-xs text-ink-3">5 fields: minute, hour, day of month, month, day of week. {tz?.differs ? <b className="font-semibold text-attn">Server time ({tz.label}), not your time zone.</b> : <>Your local time · {browserZone}</>}</p>
    <b className="text-[11.5px] font-[650] text-ink-2">Run only if</b>
    <label className="flex items-center gap-2 text-xs"><input type="checkbox" className="accent-teal" checked={previous} onChange={(e) => onChange({ ...draft, conditions: [...(e.target.checked ? [{ kind: "previous_run_finished" as const }] : []), ...quotas] })} />The previous run is no longer Running</label>
    {quotas.map((q, i) => <div key={i} className="flex flex-wrap items-center gap-2 text-xs">
      <span>{harness ? name(harness) : "The agent"} has at least</span>
      <input aria-label="Minimum quota left in percent" className="field-input w-16" type="number" min={1} max={100} step={1} value={q.min_left_percent || ""} onChange={(e) => setQuota(i, { ...q, min_left_percent: Number(e.target.value) })} />
      <span>% left of its</span>
      <select aria-label="Quota window" className="field-input w-auto" value={q.window} onChange={(e) => setQuota(i, { ...q, window: e.target.value as "five_hour" | "weekly" })}><option value="five_hour">5-hour quota</option><option value="weekly">weekly quota</option></select>
      <button className="btn btn-sm" onClick={() => setQuota(i, null)}>Remove</button>
    </div>)}
    {supportsQuota && quotas.length < 2 && <span><button className="btn btn-sm" onClick={() => setQuota(quotas.length, { kind: "quota_left", window: quotas.some((q) => q.window === "five_hour") ? "weekly" : "five_hour", min_left_percent: 20 })}>Add quota condition</button></span>}
    {!supportsQuota && !!quotas.length && <p role="alert" className="text-xs text-gh-closed">This harness has no monitored quota. Remove the quota conditions to enable this cron.</p>}
    {badPercent && <p role="alert" className="text-xs text-gh-closed">Enter a quota share from 1% to 100%.</p>}
    <p className="text-xs text-ink-3">A run whose conditions fail is skipped until the next scheduled time. Runs missed by more than 1 hour while the daemon was stopped are skipped. A quota condition needs a quota reading from the last 15 minutes.</p>
  </div>;
}

export function CronSelector({ cron, harness, onChange, onValidityChange }: { cron: CronDraft | null; harness: HarnessKind | null; onChange: (cron: CronDraft | null) => void; onValidityChange: (valid: boolean) => void }) {
  const supportsQuota = useHarnessInfo(harness)?.capabilities.quota ?? false;
  useEffect(() => { if (!cron) onValidityChange(true); }, [cron, onValidityChange]);
  return <div className="grid gap-1.5">
    <label className="text-[11.5px] font-[650] text-ink-2" htmlFor="launch-repeat">Repeat</label>
    <select id="launch-repeat" className="field-input h-8" value={cron ? "cron" : "once"} onChange={(e) => onChange(e.target.value === "cron" ? { ...DEFAULT_CRON, conditions: DEFAULT_CRON.conditions.filter((c) => supportsQuota || c.kind !== "quota_left") } : null)}>
      <option value="once">Run once</option><option value="cron">On a cron schedule…</option>
    </select>
    {cron && <div className="grid gap-2 rounded-lg border border-line bg-chrome p-2.5">
      <CronFields draft={cron} harness={harness} onChange={onChange} onValidityChange={onValidityChange} />
      <p className="text-xs text-ink-3">Each run starts a new agent with this prompt. Runs appear in Running, Done and Archived like other agents; the cron itself is listed in Cron.</p>
    </div>}
  </div>;
}
