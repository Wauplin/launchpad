import { useInbox } from "@/features/inbox/data";
import { Fragment, useEffect, useState } from "react";
import type { HarnessStatus } from "@/gen/HarnessStatus";
import type { QuotaStatus } from "@/gen/QuotaStatus";
import type { QuotaWindow } from "@/gen/QuotaWindow";
import type { Status } from "@/gen/Status";
import type { ProfileSpend } from "@/gen/ProfileSpend";
import { spendLabel, spendPercent, useQuotas, windowLabel, type QuotaEntry } from "@/lib/profiles";
import { HarnessMark, useHarnessInfo, useHarnesses, type Harness } from "@/components/HarnessMark";
import { formatAge, formatReset, resetLabel } from "@/lib/format";
import { onConnectionChange } from "@/lib/ws";
import { Popover } from "@/components/Popover";
import { TerminalsChip } from "@/features/terminals/TerminalDock";
import { isDesktopShell } from "@/lib/platform";
import { HostChip } from "./Connection";
import { Link } from "@tanstack/react-router";
import { linkLabel, useHosts } from "@/lib/hosts";

/** Compact health of the SSH hosts; quota meters stay per account, not per machine. */
function Hosts() {
  const hosts = useHosts();
  if (hosts.length === 0) return null;
  return (
    <Link to="/settings/hosts" className="inline-flex items-center gap-1.5 rounded px-1 hover:bg-select" title="Settings → Machines">
      <span className="text-ink-3">Hosts:</span>
      {hosts.map((h) => (
        <span key={h.host.id} className="inline-flex items-center gap-1" title={`${h.host.id}: ${linkLabel[h.state]}${h.error ? ` · ${h.error}` : ""}`}>
          <i className={`inline-block size-1.5 rounded-full ${h.state === "connected" ? "bg-gh-open" : h.state === "connecting" ? "bg-attn" : "bg-gh-closed"}`} aria-hidden />
          <span className={h.state === "connected" ? "" : "font-semibold text-gh-closed"}>{h.host.id}</span>
        </span>
      ))}
    </Link>
  );
}

import { useStatus } from "@/lib/queries";
export { useStatus };

function Meter({ value, harness }: { value: number; harness: Harness }) {
  const info = useHarnessInfo(harness);
  const color = value >= 90 ? "bg-gh-closed" : "";
  return (
    <span className="inline-block h-1.5 w-[34px] overflow-hidden rounded-[3px] bg-meter">
      <i className={`block h-full rounded-[3px] ${color}`} style={{ width: `${Math.min(100, Math.max(2, value))}%`, backgroundColor: value >= 90 ? undefined : info?.color }} />
    </span>
  );
}

/** The name of a login: its profiles, else the harness (sessions launched without a profile). */
export function useQuotaName(e: QuotaEntry) {
  const info = useHarnessInfo(e.harness);
  return e.names.length ? e.names.join(" · ") : (info?.name ?? e.harness);
}

function Pair({ harness, name, s, labeled }: { harness: Harness; name: string; s: QuotaStatus; labeled: boolean }) {
  const w = (label: string, q: QuotaWindow | null) => (q ? `${label}: ${Math.round(q.used_percent)}% used, ${q.resets_at === null ? "not started" : `resets ${new Date(q.resets_at * 1000).toLocaleString()}`}` : `${label}: unknown`);
  const pct = (q: QuotaWindow | null) => (q ? `${Math.round(q.used_percent)}%` : "?");
  return (
    <span className="inline-flex items-center gap-1.5" title={[name, w("5 hours", s.five_hour), w("Week", s.weekly), s.quota_error].filter(Boolean).join("\n")}>
      <HarnessMark harness={harness} size={14} />
      {labeled && <span className="max-w-[90px] truncate text-ink-3">{name}</span>}
      <Meter value={s.five_hour?.used_percent ?? 0} harness={harness} />
      <Meter value={s.weekly?.used_percent ?? 0} harness={harness} />
      <span className="text-ink-3">{pct(s.five_hour)} · {pct(s.weekly)}</span>
    </span>
  );
}

/** Rolling spending limits of a pay-as-you-go profile, shown like subscription quota meters. */
function SpendPair({ v }: { v: ProfileSpend }) {
  return (
    <Popover up width={300} trigger={({ open, toggle }) => (
      <button type="button" className="inline-flex h-[22px] items-center gap-1.5 rounded px-1 hover:bg-select focus-visible:outline-2 focus-visible:outline-teal" aria-label={`${v.name} spending limits`} aria-expanded={open} onClick={toggle}
        title={[v.name, ...v.windows.map((w) => `${windowLabel(w.hours)}: ${spendLabel(w)}`)].join("\n")}>
        <HarnessMark harness={v.harness} size={14} />
        <span className="max-w-[90px] truncate text-ink-3">{v.name}</span>
        {v.windows.map((w, i) => <Meter key={i} value={spendPercent(w)} harness={v.harness} />)}
        <span className={v.windows.some((w) => w.used_usd >= w.limit_usd) ? "font-semibold text-gh-closed" : "text-ink-3"}>{v.windows.map((w) => `${Math.round(spendPercent(w))}%`).join(" · ")}</span>
      </button>
    )}>
      {() => (
        <div className="grid gap-1 p-1.5 text-[12px] whitespace-normal">
          <span className="inline-flex items-center gap-1.5 font-semibold"><HarnessMark harness={v.harness} size={14} />{v.name} <span className="font-normal text-ink-3">· pay as you go</span></span>
          {v.windows.map((w, i) => <span key={i} className={w.used_usd >= w.limit_usd ? "text-gh-closed" : "text-ink-2"}>{windowLabel(w.hours)}: {spendLabel(w)}</span>)}
          <span className="text-ink-3">Rolling windows over the cost reported by the harness for all sessions of this profile.</span>
          <Link to="/settings/profiles" className="text-teal-ink hover:underline">Edit limits in Profiles</Link>
        </div>
      )}
    </Popover>
  );
}

/** Warning tokens, shown only when something needs attention. */
function Warnings({ harness, s }: { harness: Harness; s: HarnessStatus }) {
  const info = useHarnessInfo(harness);
  const name = info?.name ?? harness;
  return (
    <>
      {s.updating_to && <span className="rounded bg-teal-soft px-1.5 font-semibold text-teal-ink" title={`Updating to ${s.updating_to} in the background`}>{name} updating</span>}
      {!s.version && <span className="rounded bg-attn-soft px-1.5 font-semibold text-gh-closed" title={s.version_error ?? undefined}>{name} not found</span>}
    </>
  );
}

function StaleQuota({ e }: { e: QuotaEntry }) {
  const name = useQuotaName(e);
  const s = e.q;
  if (s.quota_at === null || Date.now() - s.quota_at <= 15 * 60_000) return null;
  return <span className="rounded bg-attn-soft px-1.5 font-semibold text-attn" title={s.quota_error ?? undefined}>{name} quota reading {formatAge(s.quota_at)} old</span>;
}

const emptyStatus: HarnessStatus = { version: null, version_error: null, updating_to: null };

function QuotaLine({ e }: { e: QuotaEntry }) {
  const name = useQuotaName(e);
  const s = e.q;
  return (
    <div className="grid gap-0.5">
      <span className="inline-flex items-center gap-1.5 font-semibold"><HarnessMark harness={e.harness} size={14} />{name}</span>
      <span className="text-ink-3">5h {s.five_hour ? `${Math.round(s.five_hour.used_percent)}% · ${resetLabel(s.five_hour.resets_at)}` : "unknown"} · week {s.weekly ? `${Math.round(s.weekly.used_percent)}% · ${resetLabel(s.weekly.resets_at)}` : "unknown"}{s.quota_at && ` · read ${formatAge(s.quota_at)} ago`}</span>
      {s.quota_error && <span className="text-gh-closed">{s.quota_error}</span>}
    </div>
  );
}

function QuotaButton({ entry, labeled, open, toggle }: { entry: QuotaEntry; labeled: boolean; open: boolean; toggle: () => void }) {
  const name = useQuotaName(entry);
  return (
    <button type="button" className="inline-flex h-[22px] items-center rounded px-1 hover:bg-select focus-visible:outline-2 focus-visible:outline-teal" aria-label={`${name} status details`} aria-expanded={open} onClick={toggle}>
      <Pair harness={entry.harness} name={name} s={entry.q} labeled={labeled} />
    </button>
  );
}

/** Opened by a quota meter pair, or by a Status button when no subscription profile is monitored. */
function Details({ entry, labeled, data, limits }: { entry: QuotaEntry | null; labeled: boolean; data: Status; limits: Record<string, { remaining: number; reset_at: number }> }) {
  const harnesses = useHarnesses();
  const quotas = useQuotas();
  return (
    <Popover up width={320} trigger={({ open, toggle }) => entry ? <QuotaButton entry={entry} labeled={labeled} open={open} toggle={toggle} /> : (
      <button type="button" className="inline-flex h-[22px] items-center rounded px-1 text-ink-3 hover:bg-select focus-visible:outline-2 focus-visible:outline-teal" aria-expanded={open} onClick={toggle}>Status</button>
    )}>
      {(close) => (
        <div className="grid gap-1.5 p-1.5 text-[12px] whitespace-normal">
          {quotas.map((e) => <QuotaLine key={e.key} e={e} />)}
          {quotas.length > 0 && <Link to="/settings/usage" className="w-fit text-teal-ink hover:underline" onClick={close}>Usage history and agent activity</Link>}
          <div className="grid gap-0.5 border-t border-line pt-1.5">
            {harnesses.map((h) => {
              const s = data.harnesses[h.id] ?? emptyStatus;
              return (
                <span key={h.id} className="inline-flex flex-wrap items-center gap-1.5"><HarnessMark harness={h.id} size={14} />{h.name} <span className="text-ink-2">{s.version ?? "not found"}</span>{s.updating_to && <span className="text-teal-ink">↑ {s.updating_to}</span>}{s.version_error && <span className="text-gh-closed">{s.version_error}</span>}</span>
              );
            })}
          </div>
          <div className="border-t border-line pt-1.5">
            {Object.entries(limits).map(([k, v]) => <div key={k}>GitHub {k}: {v.remaining} calls left, resets {formatReset(v.reset_at)}</div>)}
            {Object.keys(limits).length === 0 && <div className="text-ink-3">GitHub rate limits not read yet.</div>}
          </div>
          <div className="border-t border-line pt-1.5">Launchpad <span className="font-mono">{data.commit.slice(0, 12)}</span> · {data.env} environment</div>
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
  const quota = useQuotas();
  // With two logins of one harness, each meter pair names its profiles.
  const labeled = (e: QuotaEntry) => quota.filter((x) => x.harness === e.harness).length > 1;

  return (
    <footer className="flex min-h-[30px] flex-none items-center gap-x-2.5 border-t border-line bg-chrome px-3 py-1 text-[12px] whitespace-nowrap text-ink-2 tabular-nums">
      {data ? (
        <>
          {quota.length === 0 && <Details entry={null} labeled={false} data={data} limits={limits} />}
          {quota.map((e, i) => <Fragment key={e.key}>{i > 0 && <span className="h-3.5 w-px bg-line" />}<Details entry={e} labeled={labeled(e)} data={data} limits={limits} /></Fragment>)}
          {data.spend.map((v) => <Fragment key={v.profile}><span className="h-3.5 w-px bg-line" /><SpendPair v={v} /></Fragment>)}
          {harnesses.map((h) => <Warnings key={h.id} harness={h.id} s={data.harnesses[h.id] ?? emptyStatus} />)}
          {quota.map((e) => <StaleQuota key={e.key} e={e} />)}
          {low.map(([k, v]) => <span key={k} className="rounded bg-attn-soft px-1.5 font-semibold text-attn" title={`Resets ${formatReset(v.reset_at)}`}>GitHub {k} {v.remaining} calls left</span>)}
          <span className="ml-auto inline-flex items-center gap-2.5">
            <Hosts />
            {isDesktopShell ? !online && <span className="font-semibold text-gh-closed">Daemon disconnected, reconnecting…</span> : <HostChip />}
            {data.env === "dev" && <span className="font-mono text-ink-3" title={`Running commit: ${data.commit}`}>{data.commit.slice(0, 7)}</span>}
            <TerminalsChip />
            {data.env === "dev" && <span className="rounded bg-attn-soft px-1.5 font-semibold text-attn">dev</span>}
          </span>
        </>
      ) : (
        <span className="text-ink-3">Connecting to the daemon…</span>
      )}
    </footer>
  );
}
