// The phone's status button: the connection dot, and an amber warning with its count when something needs a look
// (connection, Home, runners, harnesses, quota, spending limits, GitHub rate limit). The sheet says why
// and links to the place that fixes it, then shows the quota meters.

import { Icon } from "@/components/Icon";
import { Link } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import { HostChip, useConnection } from "@/app/Connection";
import { HarnessMark, useHarnesses } from "@/components/HarnessMark";
import type { QuotaWindow } from "@/gen/QuotaWindow";
import { useInbox } from "@/features/inbox/data";
import { ago, formatReset, resetLabel, when } from "@/lib/format";
import { useStatus } from "@/lib/queries";
import { spendLabel, spendPercent, useProfiles, useQuotas, windowLabel } from "@/lib/profiles";
import { outdatedLabel, useOfflineRunners, useOutdatedRunners } from "@/lib/runners";
import { Sheet } from "./Sheet";

type Warning = { key: string; text: string; to?: string };

/** What needs a look, most serious first. Empty when all is well. */
function useStatusWarnings(): Warning[] {
  const { data } = useStatus();
  const { online, since } = useConnection();
  const harnesses = useHarnesses();
  const quotas = useQuotas();
  const profiles = useProfiles();
  const outdated = useOutdatedRunners().map((r) => r.runner.name);
  const offline = useOfflineRunners();
  const limits = useInbox().data?.limits ?? {};
  const out: Warning[] = [];
  if (!online) out.push({ key: "offline", text: `Not connected to ${data?.host || "the daemon"}${since ? ` since ${when(since)}` : ""}. Showing what was last received.` });
  if (!data) return out;
  if (data.role === "runner" && data.home && data.home.state !== "connected") out.push({ key: "home", text: `Home is not reachable (${data.home.state.replace("_", " ")})${data.home.message ? `: ${data.home.message}` : ""}.`, to: "/settings/runners" });
  if (data.role === "runner" && data.home?.compat === "outdated") out.push({ key: "self-outdated", text: "This runner needs an update: Home sends it no new sessions.", to: "/settings/runners" });
  if (outdated.length) out.push({ key: "outdated", text: `${outdatedLabel(outdated)}: no new sessions until then.`, to: "/settings/runners" });
  if (offline.length) out.push({ key: "runners-offline", text: `${offline.join(", ")} ${offline.length === 1 ? "is" : "are"} offline.`, to: "/settings/runners" });
  // Home runs no agents: its runners report their own harnesses. Only harnesses a profile uses count as missing.
  if (data.role !== "home") for (const h of harnesses) if (profiles.some((p) => p.harness === h.id) && !data.harnesses[h.id]?.version) out.push({ key: `harness-${h.id}`, text: `${h.name} is not found on this machine.`, to: "/setup" });
  for (const e of quotas) {
    const name = e.names.length ? e.names.join(" · ") : (harnesses.find((h) => h.id === e.harness)?.name ?? e.harness);
    const full = [["5 hours", e.q.five_hour], ["weekly", e.q.weekly]].filter(([, w]) => w && (w as QuotaWindow).used_percent >= 90).map(([l]) => l);
    if (full.length) out.push({ key: `quota-${e.key}`, text: `${name}: ${full.join(" and ")} quota over 90% used.` });
    if (e.q.quota_error) out.push({ key: `quota-error-${e.key}`, text: `${name}: quota reading unavailable.` });
    else if (e.q.quota_at != null && Date.now() - e.q.quota_at > 15 * 60_000) out.push({ key: `quota-stale-${e.key}`, text: `${name}: quota read ${ago(e.q.quota_at)}.` });
  }
  for (const v of data.spend) if (v.windows.some((w) => w.used_usd >= w.limit_usd)) out.push({ key: `spend-${v.profile}`, text: `${v.name} reached a spending limit.` });
  for (const [k, v] of Object.entries(limits)) if (v.remaining < 500 && v.reset_at * 1000 > Date.now()) out.push({ key: `github-${k}`, text: `GitHub ${k}: ${v.remaining} calls left, resets ${formatReset(v.reset_at)}.` });
  return out;
}

function WindowUsage({ label, quota, color }: { label: string; quota: QuotaWindow | null; color: string }) {
  return <div className="grid gap-1.5">
    <div className="flex flex-wrap justify-between gap-x-3 gap-y-1">
      <span>{label}</span>
      <span className="font-semibold tabular-nums">{quota ? `${Math.round(quota.used_percent)}% used` : "Unknown"}</span>
    </div>
    {quota && <>
      <div role="meter" aria-label={`${label} quota used`} aria-valuemin={0} aria-valuemax={100} aria-valuenow={quota.used_percent} className="h-2 overflow-hidden rounded-full bg-meter">
        <div className="h-full rounded-full" style={{ width: `${Math.min(100, Math.max(0, quota.used_percent))}%`, backgroundColor: quota.used_percent >= 90 ? "var(--color-gh-closed)" : color }} />
      </div>
      <span className="text-[12px] text-ink-3">{resetLabel(quota.resets_at)}</span>
    </>}
  </div>;
}

/** Quotas are global to the daemon, independent of the selected workspace. */
function StatusDetails() {
  const { data, error } = useStatus();
  const allHarnesses = useHarnesses();
  const quotas = useQuotas();
  const [, tick] = useState(0);
  useEffect(() => {
    const timer = setInterval(() => tick((n) => n + 1), 30_000);
    return () => clearInterval(timer);
  }, []);
  const warnings = useStatusWarnings();
  return <div className="grid gap-4 p-4 text-[14px]">
    <HostChip />
    {warnings.length > 0 && <section aria-label="Warnings" className="grid gap-1 rounded-lg border border-attn-line bg-attn-soft p-3">
      {warnings.map((w) => <div key={w.key} className="flex min-h-11 items-center gap-2 text-[13px]">
        <Icon name="alert" size={14} className="shrink-0 text-attn" />
        <span className="min-w-0 flex-1 break-words">{w.text}</span>
        {w.to && <Link to={w.to} className="btn btn-sm shrink-0 no-underline">Open</Link>}
      </div>)}
    </section>}
    {!data ? <p className="text-ink-3">{error ? "Server status is unavailable. Waiting to reconnect…" : "Loading quota status…"}</p> : <>
      <h2 className="font-semibold">Subscription quotas</h2>
      {quotas.length === 0 && <p className="text-ink-3">No subscription profile is monitored.</p>}
      {quotas.map((e) => {
        const s = e.q;
        const h = allHarnesses.find((x) => x.id === e.harness);
        const name = e.names.length ? e.names.join(" · ") : (h?.name ?? e.harness);
        const color = h?.color ?? "var(--color-teal)";
        const stale = s.quota_at != null && Date.now() - s.quota_at > 15 * 60_000;
        return <section key={e.key} aria-label={`${name} quota`} className="grid gap-3 rounded-lg border border-line p-3">
          <h3 className="flex items-center gap-2 font-semibold"><HarnessMark harness={e.harness} size={18} />{name}</h3>
          <WindowUsage label="5 hours" quota={s.five_hour} color={color} />
          <WindowUsage label="Weekly" quota={s.weekly} color={color} />
          <p className={`text-[12px] ${stale ? "font-semibold text-attn" : "text-ink-3"}`}>{s.quota_at != null ? `${stale ? "Stale reading · " : ""}read ${ago(s.quota_at)}` : "Quota not read yet."}</p>
          {s.quota_error && <p className="break-words text-[12px] text-gh-closed">Quota reading unavailable: {s.quota_error}</p>}
        </section>;
      })}
      {data.spend.length > 0 && <h2 className="font-semibold">Pay-as-you-go spending limits</h2>}
      {data.spend.map((v) => {
        const color = allHarnesses.find((h) => h.id === v.harness)?.color ?? "var(--color-teal)";
        return <section key={v.profile} aria-label={`${v.name} spending`} className="grid gap-3 rounded-lg border border-line p-3">
          <h3 className="flex items-center gap-2 font-semibold"><HarnessMark harness={v.harness} size={18} />{v.name}</h3>
          {v.windows.map((w, i) => <div key={i} className="grid gap-1.5">
            <div className="flex flex-wrap justify-between gap-x-3 gap-y-1"><span>{windowLabel(w.hours)}</span><span className="font-semibold tabular-nums">{Math.round(spendPercent(w))}% used</span></div>
            <div role="meter" aria-label={`${windowLabel(w.hours)} spending`} aria-valuemin={0} aria-valuemax={100} aria-valuenow={spendPercent(w)} className="h-2 overflow-hidden rounded-full bg-meter">
              <div className="h-full rounded-full" style={{ width: `${spendPercent(w)}%`, backgroundColor: spendPercent(w) >= 90 ? "var(--color-gh-closed)" : color }} />
            </div>
            <span className="text-[12px] text-ink-3">{spendLabel(w)}</span>
          </div>)}
        </section>;
      })}
      <p className="break-words text-[12px] text-ink-3">Launchpad <span className="font-mono">{data.commit.slice(0, 12)}</span> · {data.env} environment</p>
    </>}
  </div>;
}

export function ServerStatus() {
  const [open, setOpen] = useState(false);
  const warnings = useStatusWarnings().length;
  return <>
    <button type="button" className="phone-connection iconbtn shrink-0" aria-label={`Server status and quotas${warnings ? `: ${warnings} warning${warnings === 1 ? "" : "s"}` : ""}`} aria-haspopup="dialog" aria-expanded={open} onClick={() => setOpen(true)}>
      {warnings
        ? <span className="inline-flex items-center gap-0.5 font-semibold text-attn"><Icon name="alert" size={16} /><span className="text-[12px] tabular-nums">{warnings}</span></span>
        : <Icon name="window" size={16} />}
      <HostChip compact iconOnly />
    </button>
    {open && <Sheet title="Server status and quotas" onClose={() => setOpen(false)}><StatusDetails /></Sheet>}
  </>;
}
