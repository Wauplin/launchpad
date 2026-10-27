import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import type { AgentRun } from "@/gen/AgentRun";
import { useHarnesses } from "@/components/HarnessMark";
import { api } from "@/lib/api";
import { useConfig } from "@/lib/queries";
import { Card } from "./SettingsView";

const compact = (value: number) => new Intl.NumberFormat(undefined, { notation: "compact", maximumFractionDigits: 1 }).format(value);
const tokenCount = (run: AgentRun) => run.input_tokens + run.cached_tokens + run.output_tokens;
const dayKey = (date: Date) => `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`;
const seriesKey = (run: AgentRun) => JSON.stringify([run.harness, run.profile]);
// Keep profiles in their harness's color family, with the default using its exact quota color.
const profileColor = (base: string, index: number) => index === 0 ? base
  : `color-mix(in srgb, ${base}, ${index % 2 ? "white" : "black"} ${Math.min(65, Math.ceil(index / 2) * 18)}%)`;

export function AgentActivity({ month, selected, onSelect, onMove, onToday }: {
  month: Date; selected: string; onSelect: (day: string) => void; onMove: (delta: number) => void; onToday: () => void;
}) {
  const harnesses = useHarnesses();
  const config = useConfig();
  const from = month.getTime();
  const until = new Date(month.getFullYear(), month.getMonth() + 1, 1).getTime();
  const query = useQuery({
    queryKey: ["agent-activity", from, until],
    queryFn: () => api.get<AgentRun[]>(`/api/analytics/runs?from=${from}&until=${until}`),
    refetchInterval: 30_000,
  });
  const [metric, setMetric] = useState<"agents" | "tokens">("agents");
  const format = (value: number) => metric === "tokens" ? compact(value) : value.toLocaleString();
  const [page, setPage] = useState(0);
  const runs = query.data ?? [];
  const series = new Map<string, { key: string; label: string; color: string; total: number }>();
  const days = Array.from({ length: new Date(month.getFullYear(), month.getMonth() + 1, 0).getDate() }, (_, i) => ({
    date: new Date(month.getFullYear(), month.getMonth(), i + 1), total: 0, counts: new Map<string, number>(),
  }));
  const profiles = new Map<string, (string | null)[]>();
  for (const harness of harnesses) {
    profiles.set(harness.id, [null, ...(config.data?.config.profiles.filter((v) => v.harness === harness.id).map((v) => v.id) ?? [])]);
  }
  for (const run of runs) {
    const ids = profiles.get(run.harness) ?? [];
    if (!ids.includes(run.profile)) ids.push(run.profile);
    profiles.set(run.harness, ids);
  }
  for (const ids of profiles.values()) ids.sort((a, b) => a === b ? 0 : a === null ? -1 : b === null ? 1 : a.localeCompare(b));
  for (const run of runs) {
    const day = days[new Date(run.started_at).getDate() - 1];
    const key = seriesKey(run);
    if (!series.has(key)) {
      const info = harnesses.find((h) => h.id === run.harness);
      const harness = info?.name ?? run.harness;
      const profile = config.data?.config.profiles.find((v) => v.harness === run.harness && v.id === run.profile)?.name ?? run.profile ?? "Default";
      series.set(key, { key, label: `${harness} · ${profile}`, color: profileColor(info?.color ?? "#4b5563", profiles.get(run.harness)!.indexOf(run.profile)), total: 0 });
    }
    const amount = metric === "tokens" ? tokenCount(run) : 1;
    series.get(key)!.total += amount;
    day.total += amount;
    day.counts.set(key, (day.counts.get(key) ?? 0) + amount);
  }
  const groups = [...series.values()].sort((a, b) => b.total - a.total || a.key.localeCompare(b.key));
  const total = groups.reduce((sum, group) => sum + group.total, 0);
  const tokens = runs.reduce((sum, run) => ({ input: sum.input + run.input_tokens, cached: sum.cached + run.cached_tokens, output: sum.output + run.output_tokens }), { input: 0, cached: 0, output: 0 });
  const stats: [number, string][] = metric === "tokens"
    ? [[total, "total tokens"], [tokens.input, "input tokens"], [tokens.cached, "cached input"], [tokens.output, "output tokens"]]
    : [[runs.length, "agents started"], [days.filter((day) => day.total > 0).length, "active days"], [Math.max(0, ...days.map((day) => day.total)), "busiest day"]];
  const peak = Math.max(1, ...days.map((day) => day.total));
  const selectedDay = days.find((day) => dayKey(day.date) === selected);
  const shownPage = Math.min(page, Math.max(0, Math.ceil(groups.length / 6) - 1));
  const dateLabel = (date: Date) => date.toLocaleDateString(undefined, { month: "short", day: "numeric" });

  return <Card title="Agent activity">
    <div className="flex flex-wrap items-center justify-between gap-3">
      <div>
        <h2 className="font-semibold">Your daily rhythm</h2>
        <p className="text-sm text-ink-3">Session starts and token usage, split by harness and profile.</p>
      </div>
      <div className="flex items-center gap-1">
        <button className="btn" aria-label="Agent activity: previous month" onClick={() => onMove(-1)}>←</button>
        <button className="btn" onClick={onToday}>Today</button>
        <button className="btn" aria-label="Agent activity: next month" onClick={() => onMove(1)}>→</button>
      </div>
    </div>
    <p className="text-sm font-semibold text-teal-ink" aria-live="polite">{month.toLocaleDateString(undefined, { month: "long", year: "numeric" })}</p>
    <div className="flex w-fit gap-1 rounded-lg border border-line bg-chrome p-1" role="group" aria-label="Activity metric">
      {(["agents", "tokens"] as const).map((value) => <button key={value} className="lp-chip" aria-pressed={metric === value} onClick={() => { setMetric(value); setPage(0); }}>{value === "agents" ? "Agents" : "Tokens"}</button>)}
    </div>
    {query.isError && <p role="alert" className="text-sm text-danger">Could not load agent activity. <button className="btn" onClick={() => void query.refetch()}>Retry</button></p>}
    {query.isPending ? <div className="rounded-lg bg-chrome p-8 text-center text-sm text-ink-3">Loading agent activity…</div> : !query.isError && <>
      <div className={`grid gap-2 rounded-lg border border-line bg-chrome p-3 sm:p-4 ${metric === "tokens" ? "grid-cols-2 sm:grid-cols-4" : "grid-cols-3"}`}>
        {stats.map(([value, label]) => <div key={label} className={`min-w-0 border-line px-1 sm:px-3 ${metric === "tokens" ? "even:border-l sm:not-first:border-l" : "not-first:border-l"}`}>
          <div className="text-2xl font-semibold tabular-nums tracking-tight sm:text-3xl" title={value.toLocaleString()}>{format(value)}</div>
          <div className="text-[11px] text-ink-3 sm:text-xs">{label}</div>
        </div>)}
      </div>
      <div className="rounded-lg border border-line px-2 pb-2 pt-4 sm:px-4">
        <div className="mb-2 flex justify-between text-[10px] text-ink-3"><span>{metric === "tokens" ? "TOKENS BY SESSION START DATE" : "DAILY STARTS"}</span><span>Peak · {format(total ? peak : 0)}</span></div>
        <div className="relative">
          <div aria-hidden="true" className="pointer-events-none absolute inset-0 flex flex-col justify-between pb-6">
            {[0, 1, 2].map((line) => <div key={line} className="border-t border-dashed border-line" />)}
          </div>
          <div className="relative grid h-44 gap-px sm:h-52 sm:gap-1" style={{ gridTemplateColumns: `repeat(${days.length}, minmax(0, 1fr))` }} role="group" aria-label={`${metric === "tokens" ? "Session tokens by start date" : "Daily agent starts"}. Select a day for its harness and profile breakdown.`}>
            {days.map((day, index) => {
              const key = dayKey(day.date);
              const description = `${dateLabel(day.date)}: ${day.total.toLocaleString()} ${metric}${groups.filter((group) => day.counts.has(group.key)).map((group) => `, ${group.label}: ${day.counts.get(group.key)}`).join("")}`;
              return <button key={key} aria-label={description} aria-pressed={selected === key} title={description} onClick={() => onSelect(key)}
                className={`group flex min-w-0 flex-col justify-end rounded-t pt-1 outline-offset-2 focus-visible:outline-2 focus-visible:outline-teal-ink ${selected === key ? "bg-select" : "hover:bg-chrome"}`}>
                <span className="flex min-h-0 w-full flex-1 flex-col justify-end px-px">
                  <span className="flex w-full flex-col-reverse overflow-hidden rounded-t-sm" style={{ height: `${day.total / peak * 100}%` }}>
                    {groups.map((group) => {
                      const count = day.counts.get(group.key) ?? 0;
                      return count > 0 && <span key={group.key} style={{ background: group.color, height: `${count / day.total * 100}%` }} />;
                    })}
                  </span>
                  {!day.total && <span className="h-px bg-line" />}
                </span>
                <span className={`flex h-6 flex-none items-center justify-center text-[9px] tabular-nums ${selected === key ? "font-bold text-teal-ink" : "text-ink-3"}`}>{index === 0 || (index + 1) % 5 === 0 ? index + 1 : ""}</span>
              </button>;
            })}
          </div>
        </div>
        <div className="mt-3 border-t border-line pt-3">
          <p className="mb-2 text-xs text-ink-3" aria-live="polite">{selectedDay ? `${dateLabel(selectedDay.date)} · ${format(selectedDay.total)} ${metric}` : "Select a bar to see that day’s counts."}</p>
          <div className="flex flex-wrap gap-x-5 gap-y-2" aria-label="Chart legend">
            {groups.slice(shownPage * 6, shownPage * 6 + 6).map((group) => {
              const count = selectedDay?.counts.get(group.key) ?? 0;
              return <span key={group.key} className="inline-flex min-w-0 items-center gap-2 text-xs" title={`${group.label}: ${count.toLocaleString()} ${metric} on ${selectedDay ? dateLabel(selectedDay.date) : "the selected day"}; ${group.total.toLocaleString()} this month`}>
                <span aria-hidden="true" className="h-2.5 w-2.5 flex-none rounded-sm" style={{ background: group.color }} />
                <span className="min-w-0 break-words">{group.label}</span>
                <b className="tabular-nums">{format(count)}</b>
              </span>;
            })}
          </div>
          {groups.length > 6 && <div className="mt-3 flex items-center justify-between gap-2 text-xs text-ink-3">
            <span>{shownPage * 6 + 1}–{Math.min(groups.length, shownPage * 6 + 6)} of {groups.length} profiles</span>
            <div className="flex gap-1"><button className="btn" disabled={shownPage === 0} onClick={() => setPage(shownPage - 1)}>Previous</button><button className="btn" disabled={(shownPage + 1) * 6 >= groups.length} onClick={() => setPage(shownPage + 1)}>Next</button></div>
          </div>}
        </div>
      </div>
      {!total && <p className="py-3 text-center text-sm text-ink-3">{metric === "tokens" ? "No reported tokens for sessions started this month." : "No agents started this month. Activity will appear here after a launch."}</p>}
    </>}
    {metric === "tokens" && <p className="rounded-lg border border-line bg-chrome p-3 text-xs leading-relaxed text-ink-2">Session token totals (input + cached input + output) are grouped by start date, including later follow-ups. Daily consumption history is unavailable; missing usage counts as zero.</p>}
    <p className="text-xs leading-relaxed text-ink-3">{metric === "agents" && "One count per saved session on its start date, including archived sessions. Follow-up turns and resumes don’t add counts. "}Everything · your timezone ({Intl.DateTimeFormat().resolvedOptions().timeZone}). Historical profiles use their saved ID if no longer configured.</p>
  </Card>;
}
