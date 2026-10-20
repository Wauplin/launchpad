import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import type { AgentRun } from "@/gen/AgentRun";
import { useHarnesses } from "@/components/HarnessMark";
import { api } from "@/lib/api";
import { useConfig } from "@/lib/queries";
import { Card } from "./SettingsView";

const dayKey = (date: Date) => `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`;
const palette = ["#258b83", "#c97a48", "#7e76c5", "#4c93bb", "#bd6387", "#929443", "#b57158", "#63997b"];
const seriesKey = (run: AgentRun) => JSON.stringify([run.harness, run.variant]);
const colorFor = (key: string) => {
  let hash = 0;
  for (const char of key) hash = (hash * 31 + char.charCodeAt(0)) >>> 0;
  return palette[hash % palette.length];
};

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
  const [page, setPage] = useState(0);
  const runs = query.data ?? [];
  const series = new Map<string, { key: string; label: string; color: string; total: number }>();
  const days = Array.from({ length: new Date(month.getFullYear(), month.getMonth() + 1, 0).getDate() }, (_, i) => ({
    date: new Date(month.getFullYear(), month.getMonth(), i + 1), total: 0, counts: new Map<string, number>(),
  }));
  for (const run of runs) {
    const day = days[new Date(run.started_at).getDate() - 1];
    const key = seriesKey(run);
    if (!series.has(key)) {
      const harness = harnesses.find((h) => h.id === run.harness)?.name ?? run.harness;
      const variant = config.data?.config.variants.find((v) => v.harness === run.harness && v.id === run.variant)?.name ?? run.variant ?? "Default";
      series.set(key, { key, label: `${harness} · ${variant}`, color: colorFor(key), total: 0 });
    }
    series.get(key)!.total++;
    day.total++;
    day.counts.set(key, (day.counts.get(key) ?? 0) + 1);
  }
  const groups = [...series.values()].sort((a, b) => b.total - a.total || a.key.localeCompare(b.key));
  const peak = Math.max(1, ...days.map((day) => day.total));
  const activeDays = days.filter((day) => day.total > 0).length;
  const selectedDay = days.find((day) => dayKey(day.date) === selected);
  const shownPage = Math.min(page, Math.max(0, Math.ceil(groups.length / 6) - 1));
  const dateLabel = (date: Date) => date.toLocaleDateString(undefined, { month: "short", day: "numeric" });

  return <Card title="Analytics · Agent activity">
    <div className="flex flex-wrap items-center justify-between gap-3">
      <div>
        <h2 className="font-semibold">Your daily rhythm</h2>
        <p className="text-sm text-ink-3">Agents started, split by harness and variant.</p>
      </div>
      <div className="flex items-center gap-1">
        <button className="btn" aria-label="Agent activity: previous month" onClick={() => onMove(-1)}>←</button>
        <button className="btn" onClick={onToday}>Today</button>
        <button className="btn" aria-label="Agent activity: next month" onClick={() => onMove(1)}>→</button>
      </div>
    </div>
    <p className="text-sm font-semibold text-teal-ink" aria-live="polite">{month.toLocaleDateString(undefined, { month: "long", year: "numeric" })}</p>
    {query.isError && <p role="alert" className="text-sm text-danger">Could not load agent activity. <button className="btn" onClick={() => void query.refetch()}>Retry</button></p>}
    {query.isPending ? <div className="rounded-lg bg-chrome p-8 text-center text-sm text-ink-3">Loading agent activity…</div> : !query.isError && <>
      <div className="grid grid-cols-3 gap-2 rounded-lg border border-line bg-chrome p-3 sm:p-4">
        {[[runs.length, "agents started"], [activeDays, "active days"], [runs.length ? peak : 0, "busiest day"]].map(([value, label]) => <div key={label} className="min-w-0 border-line px-1 not-first:border-l sm:px-3">
          <div className="text-2xl font-semibold tabular-nums tracking-tight sm:text-3xl">{value}</div>
          <div className="text-[11px] text-ink-3 sm:text-xs">{label}</div>
        </div>)}
      </div>
      <div className="rounded-lg border border-line px-2 pb-2 pt-4 sm:px-4">
        <div className="mb-2 flex justify-between text-[10px] text-ink-3"><span>DAILY STARTS</span><span>Peak · {runs.length ? peak : 0}</span></div>
        <div className="relative">
          <div aria-hidden="true" className="pointer-events-none absolute inset-0 flex flex-col justify-between pb-6">
            {[0, 1, 2].map((line) => <div key={line} className="border-t border-dashed border-line" />)}
          </div>
          <div className="relative grid h-44 gap-px sm:h-52 sm:gap-1" style={{ gridTemplateColumns: `repeat(${days.length}, minmax(0, 1fr))` }} role="group" aria-label="Daily agent starts. Select a day for its harness and variant breakdown.">
            {days.map((day, index) => {
              const key = dayKey(day.date);
              const description = `${dateLabel(day.date)}: ${day.total} agents${groups.filter((group) => day.counts.has(group.key)).map((group) => `, ${group.label}: ${day.counts.get(group.key)}`).join("")}`;
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
        <p className="mt-2 text-xs text-ink-3">Select a bar to explore that day.</p>
      </div>
      {!runs.length && <p className="py-3 text-center text-sm text-ink-3">No agents started this month. Activity will appear here after a launch.</p>}
      {groups.length > 0 && <div className="rounded-lg border border-line bg-chrome p-3 sm:p-4">
        <div className="mb-3 flex flex-wrap items-baseline justify-between gap-1" aria-live="polite">
          <h3 className="text-sm font-semibold">{selectedDay ? dateLabel(selectedDay.date) : "Select a day"} <span className="font-normal text-ink-3">· {selectedDay?.total ?? 0} agents</span></h3>
          <span className="text-[11px] text-ink-3">Day / Month</span>
        </div>
        <div className="grid gap-3">
          {groups.slice(shownPage * 6, shownPage * 6 + 6).map((group) => {
            const count = selectedDay?.counts.get(group.key) ?? 0;
            return <div key={group.key} className="min-w-0">
              <div className="mb-1 flex items-center gap-2 text-xs">
                <span aria-hidden="true" className="h-2.5 w-2.5 flex-none rounded-full" style={{ background: group.color }} />
                <span className="min-w-0 flex-1 break-words">{group.label}</span>
                <span className="flex-none tabular-nums"><b>{count}</b><span className="text-ink-3"> / {group.total}</span></span>
              </div>
              <div className="h-1 overflow-hidden rounded-full bg-line"><div className="h-full rounded-full" style={{ width: `${count / Math.max(1, selectedDay?.total ?? 0) * 100}%`, background: group.color }} /></div>
            </div>;
          })}
        </div>
        {groups.length > 6 && <div className="mt-3 flex items-center justify-between gap-2 text-xs text-ink-3">
          <span>{shownPage * 6 + 1}–{Math.min(groups.length, shownPage * 6 + 6)} of {groups.length} variants</span>
          <div className="flex gap-1"><button className="btn" disabled={shownPage === 0} onClick={() => setPage(shownPage - 1)}>Previous</button><button className="btn" disabled={(shownPage + 1) * 6 >= groups.length} onClick={() => setPage(shownPage + 1)}>Next</button></div>
        </div>}
      </div>}
    </>}
    <p className="text-xs leading-relaxed text-ink-3">One count per saved session on its start date, including archived sessions. Follow-up turns and resumes don’t add counts. All workspaces · your timezone ({Intl.DateTimeFormat().resolvedOptions().timeZone}). Historical variants use their saved ID if no longer configured.</p>
  </Card>;
}
