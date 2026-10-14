import { Icon } from "@/components/Icon";
import { useEffect, useState } from "react";
import { HostChip, useConnection } from "@/app/Connection";
import { HarnessMark, useHarnesses } from "@/components/HarnessMark";
import type { QuotaWindow } from "@/gen/QuotaWindow";
import { formatAge, resetLabel } from "@/lib/format";
import { useStatus } from "@/lib/queries";
import { Sheet } from "./Sheet";

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
  const harnesses = useHarnesses().filter((h) => h.capabilities.quota);
  const { online } = useConnection();
  const [, tick] = useState(0);
  useEffect(() => {
    const timer = setInterval(() => tick((n) => n + 1), 30_000);
    return () => clearInterval(timer);
  }, []);
  return <div className="grid gap-4 p-4 text-[14px]">
    <HostChip />
    {!online && <p className="text-attn">Offline · showing the last received quota readings.</p>}
    {!data ? <p className="text-ink-3">{error ? "Server status is unavailable. Waiting to reconnect…" : "Loading quota status…"}</p> : <>
      <h2 className="font-semibold">Subscription quotas</h2>
      {harnesses.length === 0 && <p className="text-ink-3">No harness reports subscription quotas.</p>}
      {harnesses.map((h) => {
        const s = data.harnesses[h.id];
        const stale = s?.quota_at != null && Date.now() - s.quota_at > 15 * 60_000;
        return <section key={h.id} aria-label={`${h.name} quota`} className="grid gap-3 rounded-lg border border-line p-3">
          <h3 className="flex items-center gap-2 font-semibold"><HarnessMark harness={h.id} size={18} />{h.name}</h3>
          <WindowUsage label="5 hours" quota={s?.five_hour ?? null} color={h.color} />
          <WindowUsage label="Weekly" quota={s?.weekly ?? null} color={h.color} />
          <p className={`text-[12px] ${stale ? "font-semibold text-attn" : "text-ink-3"}`}>{s?.quota_at != null ? `${stale ? "Stale reading · " : ""}read ${formatAge(s.quota_at)} ago` : "Quota not read yet."}</p>
          {s?.quota_error && <p className="break-words text-[12px] text-gh-closed">Quota reading unavailable: {s.quota_error}</p>}
        </section>;
      })}
      <p className="break-words text-[12px] text-ink-3">Launchpad <span className="font-mono">{data.commit.slice(0, 12)}</span> · {data.profile} profile</p>
    </>}
  </div>;
}

export function ServerStatus() {
  const [open, setOpen] = useState(false);
  return <>
    <button type="button" className="phone-connection iconbtn shrink-0" aria-label="Server status and quotas" aria-haspopup="dialog" aria-expanded={open} onClick={() => setOpen(true)}>
      <Icon name="window" size={16} /><HostChip compact iconOnly />
    </button>
    {open && <Sheet title="Server status and quotas" onClose={() => setOpen(false)}><StatusDetails /></Sheet>}
  </>;
}
