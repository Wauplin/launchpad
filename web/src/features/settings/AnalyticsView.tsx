import { useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import type { WeeklyReset } from "@/gen/WeeklyReset";
import { api } from "@/lib/api";
import { useStatus } from "@/lib/queries";
import { useHarnesses } from "@/components/HarnessMark";
import { usePhone } from "@/lib/phone";
import { useConnection } from "@/app/Connection";
import { AgentActivity } from "./AgentActivity";
import { Card, SettingsPage } from "./SettingsView";

const dayKey = (date: Date) => `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`;
const timestamp = (ms: number) => new Date(ms).toLocaleString();

export function AnalyticsView() {
  const harnesses = useHarnesses().filter((h) => h.capabilities.quota);
  const names: Record<string, string> = Object.fromEntries(harnesses.map((h) => [h.id, h.name]));
  const colors: Record<string, string> = Object.fromEntries(harnesses.map((h) => [h.id, h.color]));
  const [month, setMonth] = useState(() => new Date(new Date().getFullYear(), new Date().getMonth(), 1));
  const [selected, setSelected] = useState(() => dayKey(new Date()));
  const from = month.getTime();
  const until = new Date(month.getFullYear(), month.getMonth() + 1, 1).getTime();
  const history = useQuery({
    queryKey: ["quota-history", from, until],
    queryFn: () => api.get<WeeklyReset[]>(`/api/quota-history?from=${from}&until=${until}`),
    refetchInterval: 30_000,
  });
  const status = useStatus();
  const phone = usePhone();
  const label = (id: string) => phone ? names[id]?.slice(0, 2) : names[id];
  const { online } = useConnection();
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(timer);
  }, []);
  const expected = Object.entries(names).map(([id, name]) => {
    const reading = status.data?.harnesses[id];
    const at = reading?.weekly?.resets_at;
    const fresh = !!reading?.quota_at && now - reading.quota_at <= 15 * 60_000 && !reading.quota_error && !status.isError && online;
    return { id, name, reading, at: at ? at * 1000 : null, fresh };
  });
  const events = history.data ?? [];
  const days = new Date(month.getFullYear(), month.getMonth() + 1, 0).getDate();
  const offset = (month.getDay() + 6) % 7;
  const selectedEvents = events.filter((event) => dayKey(new Date(event.observed_at)) === selected);
  const selectedExpected = expected.filter((item) => item.at && dayKey(new Date(item.at)) === selected);
  const move = (delta: number) => {
    const date = new Date(month.getFullYear(), month.getMonth() + delta, 1);
    setMonth(date);
    setSelected(dayKey(date));
  };

  return <SettingsPage>
    <AgentActivity month={month} selected={selected} onSelect={setSelected} onMove={move} onToday={() => { const today = new Date(); setMonth(new Date(today.getFullYear(), today.getMonth(), 1)); setSelected(dayKey(today)); }} />
    <Card title="Analytics · Weekly quota resets">
      <p className="text-sm text-ink-2">Observed resets come from drops in weekly usage, including early resets. Expected dates are the providers’ latest predictions and can change.</p>
      <details className="text-xs text-ink-3"><summary className="cursor-pointer py-2">How resets are recorded · {Intl.DateTimeFormat().resolvedOptions().timeZone}</summary><p>History starts when this version runs. Calendar dates use your timezone ({Intl.DateTimeFormat().resolvedOptions().timeZone}). A reset is placed on the day it was detected; the details show the interval between readings. Corrections to reported usage can also appear as drops. Resets with no observed usage drop cannot be detected.</p></details>
      <div className="grid gap-2 sm:grid-cols-2">
        {expected.map(({ id, name, reading, at, fresh }) => <div key={id} className="min-w-0 rounded border border-line p-3 text-sm">
          <b style={{ color: colors[id] }}>{name}</b>
          <p className="break-words">{at ? `Expected: ${timestamp(at)}` : "Next reset unknown"}</p>
          <p className="text-xs text-ink-3">{!online ? "Offline · " : reading?.quota_error ? "Reading unavailable · " : !fresh ? "No fresh reading · " : ""}{at && at <= now ? "Prediction elapsed; awaiting a new reading · " : ""}{reading?.quota_at ? `Last read ${timestamp(reading.quota_at)}` : "Waiting for quota monitoring"}</p>
        </div>)}
      </div>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h2 className="font-semibold" aria-live="polite">{month.toLocaleDateString(undefined, { month: "long", year: "numeric" })}</h2>
        <div className="flex gap-1">
          <button className="btn" aria-label="Previous month" onClick={() => move(-1)}>←</button>
          <button className="btn" onClick={() => { const today = new Date(); setMonth(new Date(today.getFullYear(), today.getMonth(), 1)); setSelected(dayKey(today)); }}>Today</button>
          <button className="btn" aria-label="Next month" onClick={() => move(1)}>→</button>
        </div>
      </div>
      {history.isError && <p role="alert" className="text-sm text-danger">Could not load reset history. <button className="btn" onClick={() => void history.refetch()}>Retry</button></p>}
      {history.isPending && <p className="text-sm text-ink-3">Loading reset history…</p>}
      <div className="grid grid-cols-7 gap-px overflow-hidden rounded border border-line bg-line" aria-label="Weekly quota reset calendar">
        {["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"].map((day) => <div key={day} className="bg-chrome py-2 text-center text-[11px] text-ink-3">{day}</div>)}
        {Array.from({ length: Math.ceil((days + offset) / 7) * 7 }, (_, index) => {
          const day = index - offset + 1;
          if (day < 1 || day > days) return <div key={index} className="bg-chrome" />;
          const date = new Date(month.getFullYear(), month.getMonth(), day);
          const key = dayKey(date);
          const observed = events.filter((event) => dayKey(new Date(event.observed_at)) === key);
          const predicted = expected.filter((item) => item.at && dayKey(new Date(item.at)) === key);
          return <button key={index} onClick={() => setSelected(key)} aria-pressed={key === selected}
            aria-label={`${date.toLocaleDateString()}: ${observed.length} observed resets, ${predicted.length} expected resets`}
            className={`flex min-h-20 min-w-0 flex-col items-start sm:min-h-24 p-1 text-left text-[10px] sm:p-2 sm:text-xs ${key === selected ? "bg-select" : "bg-paper"}`}>
            <span className={`mb-1 block text-xs ${key === dayKey(new Date()) ? "font-bold underline" : ""}`}>{day}</span>
            {Object.keys(names).map((id) => {
              const count = observed.filter((event) => event.harness === id).length;
              return count > 0 && <span key={id} className="block break-words font-semibold" style={{ color: colors[id] }}>{label(id)} ✓{count > 1 ? ` ×${count}` : ""}</span>;
            })}
            {predicted.map((item) => <span key={item.id} className="block break-words" style={{ color: colors[item.id] }}>{label(item.id)} ◇{!item.fresh || item.at! <= now ? "?" : ""}</span>)}
          </button>;
        })}
      </div>
      <p className="text-xs text-ink-3">✓ Observed usage drop · ◇ Expected reset · ? Stale or elapsed prediction{phone && harnesses.map((h) => ` · ${label(h.id)}: ${h.name}`).join("")}</p>
      {!history.isPending && !history.isError && events.length === 0 && <p className="text-sm text-ink-3">No observed resets recorded this month.</p>}
      <div className="grid gap-2 rounded border border-line p-3 text-sm" aria-live="polite">
        <h3 className="font-semibold">Selected day</h3>
        {selectedEvents.slice(-10).map((event) => <p key={`${event.harness}-${event.observed_at}`} className="break-words">
          <b style={{ color: colors[event.harness] }}>{names[event.harness] ?? event.harness}</b>: observed {timestamp(event.observed_at)} ({event.previous_percent.toFixed(1)}% → {event.used_percent.toFixed(1)}%). Reset detected between {timestamp(event.previous_at)} and {timestamp(event.observed_at)}.
        </p>)}
        {selectedEvents.length > 10 && <p>Showing the latest 10 of {selectedEvents.length} drops this day.</p>}
        {selectedExpected.map((item) => <p key={item.id}>{item.name}: expected {timestamp(item.at!)}{!item.fresh ? " (stale reading)" : ""}.</p>)}
        {!selectedEvents.length && !selectedExpected.length && <p className="text-ink-3">No resets to show for this day.</p>}
      </div>
    </Card>
  </SettingsPage>;
}
