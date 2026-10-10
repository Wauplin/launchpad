import { useInbox } from "@/features/inbox/InboxView";
import { useQuery } from "@tanstack/react-query";
import { Fragment, useEffect, useState } from "react";
import type { HarnessStatus } from "@/gen/HarnessStatus";
import type { QuotaWindow } from "@/gen/QuotaWindow";
import type { Status } from "@/gen/Status";
import { HarnessMark, useHarnessInfo, useHarnesses, type Harness } from "@/components/HarnessMark";
import { api } from "@/lib/api";
import { formatAge, formatReset } from "@/lib/format";
import { onConnectionChange } from "@/lib/ws";
import { useSessions } from "@/lib/queries";
import { isDone, isRunning, isQuotaPaused } from "@/lib/sessionStatus";
import { useScope } from "@/lib/workspace";
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

function Window({ label, w, harness }: { label: string; w: QuotaWindow | null; harness: Harness }) {
  if (!w) {
    return (
      <span className="inline-flex items-center gap-1.5 text-ink-3">
        {label} <span className="inline-block h-1.5 w-[34px] rounded-[3px] bg-meter" /> unknown
      </span>
    );
  }
  const pct = Math.round(w.used_percent);
  return (
    <span className="inline-flex items-center gap-1.5" title={`Resets ${new Date(w.resets_at * 1000).toLocaleString()}`}>
      {label} <Meter value={pct} harness={harness} />
      {pct}% · {formatReset(w.resets_at)}
    </span>
  );
}

function HarnessPart({ harness, s }: { harness: Harness; s: HarnessStatus }) {
  const info = useHarnessInfo(harness);
  const name = info?.name ?? harness;
  const stale = s.quota_at !== null && Date.now() - s.quota_at > 15 * 60_000;
  return (
    <>
      <span className="inline-flex items-center gap-1.5 font-semibold text-ink" title={s.version_error ?? undefined}>
        <HarnessMark harness={harness} size={15} />
        {name} {s.version ?? <span className="font-normal text-gh-closed">not found</span>}
        {s.updating_to && (
          <span className="text-teal-ink" title="Updating in the background">
            ↑ {s.updating_to === "latest" ? "updating" : s.updating_to}
          </span>
        )}
      </span>
      {info?.capabilities.quota && <span className="contents" title={s.quota_error ?? undefined}>
        <Window label="5h" w={s.five_hour} harness={harness} />
        <Window label="week" w={s.weekly} harness={harness} />
      </span>}
      {stale && s.quota_at && <span className="text-ink-3" title={s.quota_error ?? undefined}>({formatAge(s.quota_at)} ago)</span>}
    </>
  );
}

export function StatusBar() {
  const harnesses = useHarnesses();
  const { data } = useStatus();
  const limits = useInbox().data?.limits;
  // A workspace counts its own agents, with the sidebar's predicates.
  const scope = useScope();
  const scoped = useSessions().data?.filter(scope.hasSession);
  const counts = scope.workspace && scoped ? { running: scoped.filter(isRunning).length, done: scoped.filter(isDone).length, quota_paused: scoped.filter(isQuotaPaused).length } : data?.sessions;
  const [online, setOnline] = useState(true);
  useEffect(() => onConnectionChange(setOnline), []);
  // Re-render every 30 s so ages and reset times stay current.
  const [, tick] = useState(0);
  useEffect(() => {
    const t = setInterval(() => tick((n) => n + 1), 30_000);
    return () => clearInterval(t);
  }, []);

  return (
    <footer className="flex min-h-[30px] flex-none flex-wrap items-center gap-x-2.5 gap-y-1 py-1 border-t border-line bg-chrome px-3 text-[11.5px] whitespace-nowrap text-ink-2 tabular-nums">
      {data ? (
        <>
          {harnesses.map((h, i) => <Fragment key={h.id}>{i > 0 && <span className="h-3.5 w-px bg-line" />}<HarnessPart harness={h.id} s={data.harnesses[h.id] ?? { version: null, version_error: null, updating_to: null, five_hour: null, weekly: null, quota_at: null, quota_error: null }} /></Fragment>)}
          <span className="text-ink-3" title={Object.entries(limits ?? {}).map(([k,v]) => `${k}: ${v.remaining} remaining, resets ${formatReset(v.reset_at)}`).join("; ")}>{Object.entries(limits ?? {}).map(([k,v]) => `GH ${k} ${v.remaining}`).join(" · ")}</span>
          <span className="ml-auto inline-flex items-center gap-3.5">
            {!online && <span className="font-semibold text-gh-closed">Daemon disconnected, reconnecting…</span>}
            <span className="inline-flex items-center gap-1.5" title={`Running commit: ${data.commit}`}>
              Launchpad <span className="font-mono">{data.commit.slice(0, 7)}</span>
            </span>
            <TerminalButton />
            {data.profile === "dev" && <span className="rounded bg-attn-soft px-1.5 font-semibold text-attn">dev profile</span>}
            <span>
              {counts?.running ?? 0} running ·{" "}
              {(counts?.quota_paused ?? 0) > 0 && <span>{counts!.quota_paused} quota paused · </span>}
              <span className={(counts?.done ?? 0) > 0 ? "font-bold text-attn" : ""}>{counts?.done ?? 0} done</span>
            </span>
          </span>
        </>
      ) : (
        <span className="text-ink-3">Connecting to the daemon…</span>
      )}
    </footer>
  );
}
