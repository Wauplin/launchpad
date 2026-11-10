// Agent CLI updates on the runners (launchpad_setup::cli_update): Home reads the newest version of each CLI on npm
// every six hours; runners install a newer release of the same major version on their own, a new major version waits
// for Approve. Settings → Home and runners shows the versions and the switch per CLI; Today shows the results.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import type { CliUpdate } from "@/gen/CliUpdate";
import type { CliUpdateRunner } from "@/gen/CliUpdateRunner";
import type { CliUpdates } from "@/gen/CliUpdates";
import { ActionButton } from "@/components/ActionButton";
import { Badge, Choice, Choices, Loading, SettingRow, type BadgeTone } from "@/components/ui";
import { toastError } from "@/components/Toast";
import { runAction } from "@/lib/actions";
import { api } from "@/lib/api";
import { ago } from "@/lib/format";
import { CommandLine } from "./RunnersView";
import { Card } from "./SettingsView";

export const useCliUpdates = () => useQuery({ queryKey: ["cli-updates"], queryFn: () => api.get<CliUpdates>("/api/cli-updates"), staleTime: 60_000 });

const STATE: Record<CliUpdateRunner["state"], { label: string; tone: BadgeTone }> = {
  approval: { label: "waits for approval", tone: "attn" },
  queued: { label: "queued", tone: "neutral" },
  requested: { label: "waits for its agents", tone: "work" },
  installing: { label: "installing", tone: "work" },
  done: { label: "updated", tone: "ok" },
  failed: { label: "failed", tone: "danger" },
  manual: { label: "update it yourself", tone: "attn" },
};

/** One line per runner of an update: its state, the version it had, why it failed and the command to run. */
export function CliUpdateRunners({ u }: { u: CliUpdate }) {
  return (
    <div className="grid gap-1.5">
      {u.runners.map((e) => (
        <div key={e.runner_id} className="grid gap-1 text-[12px]">
          <span className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
            <b className="font-[650]">{e.runner}</b>
            <span className="text-ink-3">{e.from ?? "?"} → {u.version}</span>
            <Badge tone={STATE[e.state].tone}>{STATE[e.state].label}{e.state === "requested" && e.running > 0 ? ` (${e.running} working)` : ""}</Badge>
            <span className="text-ink-3">{ago(e.at)}</span>
          </span>
          {e.error && e.state !== "done" && <span className="text-ink-2">{e.error}</span>}
          {e.state === "manual" && e.command && <CommandLine command={e.command} />}
        </div>
      ))}
    </div>
  );
}

/** Today's details of an update: the runners and the release notes. */
export function CliUpdateDetails({ id }: { id: string }) {
  const q = useCliUpdates();
  const u = q.data?.updates.find((x) => x.id === id);
  if (!u) return q.isLoading ? <Loading what="the update" /> : null;
  return (
    <div className="grid gap-2 rounded-card border border-line bg-paper p-3">
      <CliUpdateRunners u={u} />
      <a className="w-fit text-[12px] underline" href={u.changelog} target="_blank" rel="noreferrer">Release notes</a>
    </div>
  );
}

/** Settings → Home and runners: the newest version of each agent CLI, the switch per CLI and recent updates. */
export function CliUpdatesCard() {
  const qc = useQueryClient();
  const q = useCliUpdates();
  const d = q.data;
  const set = (r: CliUpdates) => qc.setQueryData(["cli-updates"], r);
  const check = <ActionButton<CliUpdates> name="cli-update.check" onDone={set} />;
  if (!d) return <Card title="Agent CLI updates"><Loading what="agent CLI updates" /></Card>;
  const recent = d.updates.slice(0, 6);
  return (
    <Card title="Agent CLI updates" tour="settings.cli-updates" end={check}>
      <span className="text-[13px] text-ink-2">
        Home reads the newest version of each agent CLI on npm every six hours{d.checked_at ? ` (last ${ago(d.checked_at)})` : ""}. Each runner installs a newer release of the same major version on its own once no agent there is in a turn; a new major version waits for your approval on Today. Burst and Home's Space use the versions of Home's image.
      </span>
      {d.harnesses.map((h) => (
        <SettingRow key={h.harness} label={h.name} hint={h.error ? `Check failed: ${h.error}` : h.auto ? (h.latest ? `Newest on npm: ${h.latest} (${h.npm})` : h.npm) : "Runners keep the version they have."}>
          <Choices label={`Update ${h.name} automatically`}>
            <Choice active={h.auto} onClick={() => !h.auto && void runAction<CliUpdates>("cli-update.auto", undefined, { harness: h.harness, on: true }).then(set, toastError)}>Automatic</Choice>
            <Choice active={!h.auto} onClick={() => h.auto && void runAction<CliUpdates>("cli-update.auto", undefined, { harness: h.harness, on: false }).then(set, toastError)}>Off</Choice>
          </Choices>
        </SettingRow>
      ))}
      {recent.length > 0 && (
        <div className="grid gap-2.5">
          <b className="text-[12px] text-ink-3">Recent updates</b>
          {recent.map((u) => (
            <div key={u.id} className="grid gap-1.5 border-t border-line-2 pt-2">
              <span className="flex flex-wrap items-center gap-2 text-[13px]">
                <b>{d.harnesses.find((h) => h.harness === u.harness)?.name ?? u.harness} {u.version}</b>
                <a className="text-[12px] underline" href={u.changelog} target="_blank" rel="noreferrer">Release notes</a>
                <span className="ml-auto flex gap-1.5">
                  {u.runners.some((e) => e.state === "approval") && <ActionButton name="cli-update.approve" params={{ id: u.id }} vars={{ version: u.version }} />}
                  {u.runners.some((e) => e.state === "failed" || e.state === "manual") && <ActionButton name="cli-update.retry" params={{ id: u.id }} vars={{ version: u.version }} />}
                </span>
              </span>
              <CliUpdateRunners u={u} />
            </div>
          ))}
        </div>
      )}
    </Card>
  );
}
