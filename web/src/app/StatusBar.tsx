import { useInbox } from "@/features/inbox/InboxView";
import { useQuery } from "@tanstack/react-query";
import { Fragment, useEffect, useState } from "react";
import type { HarnessStatus } from "@/gen/HarnessStatus";
import type { QuotaWindow } from "@/gen/QuotaWindow";
import type { Status } from "@/gen/Status";
import { HarnessMark, useHarnessInfo, useHarnesses, type Harness } from "@/components/HarnessMark";
import { api } from "@/lib/api";
import { formatAge, formatReset, resetLabel } from "@/lib/format";
import { onConnectionChange } from "@/lib/ws";
import { Popover } from "@/components/Popover";
import { TerminalButton } from "@/features/terminals/TerminalDock";

export function useStatus() {
  return useQuery({ queryKey: ["status"], queryFn: () => api.get<Status>("/api/status"), staleTime: Infinity });
}

function Meter({ value, harness }: { value: number; harness: Harness }) {
  const info = useHarnessInfo(harness);
  const color = value >= 90 ? "bg-gh-closed" : "";
  return (
    <span className="inline-block h-1.5 w-[34px] overflow-hidden rounded-[3px] bg-meter">
      <i className={`block h-full rounded-[3px] ${color}`} style={{ width: `${Math.min(100, Math.max(2, value))}%`, backgroundColor: value >= 90 ? undefined : info?.color }} />
    </span>
  );
}

function Pair({ harness, s }: { harness: Harness; s: HarnessStatus }) {
  const info = useHarnessInfo(harness);
  const name = info?.name ?? harness;
  const w = (label: string, q: QuotaWindow | null) => (q ? `${label}: ${Math.round(q.used_percent)}% used, ${q.resets_at === null ? "not started" : `resets ${new Date(q.resets_at * 1000).toLocaleString()}`}` : `${label}: unknown`);
  const pct = (q: QuotaWindow | null) => (q ? `${Math.round(q.used_percent)}%` : "?");
  return (
    <span className="inline-flex items-center gap-1.5" title={[name, w("5 hours", s.five_hour), w("Week", s.weekly), s.quota_error].filter(Boolean).join("\n")}>
      <HarnessMark harness={harness} size={14} />
      <Meter value={s.five_hour?.used_percent ?? 0} harness={harness} />
      <Meter value={s.weekly?.used_percent ?? 0} harness={harness} />
      <span className="text-ink-3">{pct(s.five_hour)} · {pct(s.weekly)}</span>
    </span>
  );
}

/** Warning tokens, shown only when something needs attention. */
function Warnings({ harness, s }: { harness: Harness; s: HarnessStatus }) {
  const info = useHarnessInfo(harness);
  const name = info?.name ?? harness;
  const stale = info?.capabilities.quota && s.quota_at !== null && Date.now() - s.quota_at > 15 * 60_000;
  return (
    <>
      {s.updating_to && <span className="rounded bg-teal-soft px-1.5 font-semibold text-teal-ink" title={`Updating to ${s.updating_to} in the background`}>{name} updating</span>}
      {!s.version && <span className="rounded bg-attn-soft px-1.5 font-semibold text-gh-closed" title={s.version_error ?? undefined}>{name} not found</span>}
      {stale && s.quota_at && <span className="rounded bg-attn-soft px-1.5 font-semibold text-attn" title={s.quota_error ?? undefined}>{name} quota reading {formatAge(s.quota_at)} old</span>}
    </>
  );
}

const emptyStatus: HarnessStatus = { version: null, version_error: null, updating_to: null, five_hour: null, weekly: null, quota_at: null, quota_error: null };

function Details({ harness, data, limits }: { harness: Harness; data: Status; limits: Record<string, { remaining: number; reset_at: number }> }) {
  const harnesses = useHarnesses();
  const info = useHarnessInfo(harness);
  return (
    <Popover up width={320} trigger={({ open, toggle }) => (
      <button type="button" className="inline-flex h-[22px] items-center rounded px-1 hover:bg-select focus-visible:outline-2 focus-visible:outline-teal" aria-label={`${info?.name ?? harness} status details`} aria-expanded={open} onClick={toggle}>
        <Pair harness={harness} s={data.harnesses[harness] ?? emptyStatus} />
      </button>
    )}>
      {() => (
        <div className="grid gap-1.5 p-1.5 text-[12px] whitespace-normal">
          {harnesses.map((h) => {
            const s = data.harnesses[h.id] ?? emptyStatus;
            return (
              <div key={h.id} className="grid gap-0.5">
                <span className="inline-flex items-center gap-1.5 font-semibold"><HarnessMark harness={h.id} size={14} />{h.name} <span className="font-normal text-ink-2">{s.version ?? "not found"}</span>{s.updating_to && <span className="font-normal text-teal-ink">↑ {s.updating_to}</span>}</span>
                {h.capabilities.quota && <span className="text-ink-3">5h {s.five_hour ? `${Math.round(s.five_hour.used_percent)}% · ${resetLabel(s.five_hour.resets_at)}` : "unknown"} · week {s.weekly ? `${Math.round(s.weekly.used_percent)}% · ${resetLabel(s.weekly.resets_at)}` : "unknown"}{s.quota_at && ` · read ${formatAge(s.quota_at)} ago`}</span>}
                {(s.version_error || s.quota_error) && <span className="text-gh-closed">{s.version_error ?? s.quota_error}</span>}
              </div>
            );
          })}
          <div className="border-t border-line pt-1.5">
            {Object.entries(limits).map(([k, v]) => <div key={k}>GitHub {k}: {v.remaining} calls left, resets {formatReset(v.reset_at)}</div>)}
            {Object.keys(limits).length === 0 && <div className="text-ink-3">GitHub rate limits not read yet.</div>}
          </div>
          <div className="border-t border-line pt-1.5">Launchpad <span className="font-mono">{data.commit.slice(0, 12)}</span> · {data.profile} profile</div>
        </div>
      )}
    </Popover>
  );
}

export function StatusBar() {
  const harnesses = useHarnesses();
  const { data } = useStatus();
  const limits = useInbox().data?.limits ?? {};
  const low = Object.entries(limits).filter(([, v]) => v.remaining < 500);
  const [online, setOnline] = useState(true);
  useEffect(() => onConnectionChange(setOnline), []);
  // Re-render every 30 s so ages and reset times stay current.
  const [, tick] = useState(0);
  useEffect(() => {
    const t = setInterval(() => tick((n) => n + 1), 30_000);
    return () => clearInterval(t);
  }, []);
  const quota = harnesses.filter((h) => h.capabilities.quota);

  return (
    <footer className="flex min-h-[30px] flex-none items-center gap-x-2.5 border-t border-line bg-chrome px-3 py-1 text-[11.5px] whitespace-nowrap text-ink-2 tabular-nums">
      {data ? (
        <>
          {quota.map((h, i) => <Fragment key={h.id}>{i > 0 && <span className="h-3.5 w-px bg-line" />}<Details harness={h.id} data={data} limits={limits} /></Fragment>)}
          {harnesses.map((h) => <Warnings key={h.id} harness={h.id} s={data.harnesses[h.id] ?? emptyStatus} />)}
          {low.map(([k, v]) => <span key={k} className="rounded bg-attn-soft px-1.5 font-semibold text-attn" title={`Resets ${formatReset(v.reset_at)}`}>GitHub {k} {v.remaining} calls left</span>)}
          <span className="ml-auto inline-flex items-center gap-2.5">
            {!online && <span className="font-semibold text-gh-closed">Daemon disconnected, reconnecting…</span>}
            {data.profile === "dev" && <span className="font-mono text-ink-3" title={`Running commit: ${data.commit}`}>{data.commit.slice(0, 7)}</span>}
            <TerminalButton />
            {data.profile === "dev" && <span className="rounded bg-attn-soft px-1.5 font-semibold text-attn">dev</span>}
          </span>
        </>
      ) : (
        <span className="text-ink-3">Connecting to the daemon…</span>
      )}
    </footer>
  );
}
