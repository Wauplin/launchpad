// The network check of a runner (launchpad_core::netcap::network_check): an owner-clicked probe in a real session
// sandbox on that runner. Agents must reach the same addresses with the same credentials on the laptop jail, a
// Local VM and Burst; a failed required check shows here and as a badge on the runner.

import { useQueryClient } from "@tanstack/react-query";
import type { NetworkCheck } from "@/gen/NetworkCheck";
import type { RunnerStatus } from "@/gen/RunnerStatus";
import { ActionButton } from "@/components/ActionButton";
import { Badge } from "@/components/ui";
import { ago } from "@/lib/format";

const METHOD: Record<string, string> = { jail: "jail", vm: "Local VM", burst: "Burst" };

/** Whether the runner's last network check found a difference. */
export function networkDiffers(r: RunnerStatus): boolean {
  return r.network_check !== null && !r.network_check.ok;
}

export function RunnerNetwork({ r }: { r: RunnerStatus }) {
  const qc = useQueryClient();
  const check: NetworkCheck | null = r.network_check;
  const failed = check ? check.items.filter((i) => !i.ok) : [];
  return (
    <div className="grid gap-1 pl-3 text-[12px] text-ink-2">
      <div className="flex flex-wrap items-center gap-2">
        <span className="min-w-0 flex-1">
          {check === null
            ? "Network not checked yet."
            : <>Network {check.ok ? <Badge tone="ok">same as everywhere</Badge> : <Badge tone="attn">differs</Badge>} <span className="text-ink-3">{METHOD[check.method] ?? check.method} · checked {ago(check.at)}</span></>}
        </span>
        <ActionButton name="runner.check-network" params={{ id: r.runner.id }} disabled={!r.connected} title={r.connected ? undefined : `${r.runner.name} is offline`} onDone={() => qc.invalidateQueries({ queryKey: ["runners"] })}>
          {check === null ? "Check network" : "Check again"}
        </ActionButton>
      </div>
      {failed.map((i) => (
        <span key={i.id} className={i.required ? "text-gh-closed" : "text-ink-3"} title={i.detail}>
          {i.required ? "✕" : "–"} {i.label}: {i.detail}
        </span>
      ))}
    </div>
  );
}
