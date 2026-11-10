// Settings → Home and runners (Home): the runners (machines that run agents), with Approve for a pairing request,
// Remove, and the command that installs and pairs a new one, then Burst and agent CLI updates. A runner's own window shows only its status
// (features/runner).

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { toast, toastError } from "@/components/Toast";
import { Badge, ConfirmButton, Empty, ErrorState, Choice, Choices, Loading, MenuSection, MoreMenu, SettingRow } from "@/components/ui";
import { Icon } from "@/components/Icon";
import { api } from "@/lib/api";
import { copyText } from "@/lib/clipboard";
import { delegatedAccess } from "@/lib/deployment";
import { ago } from "@/lib/format";
import { useSessions, useStatus } from "@/lib/queries";
import { useRunners } from "@/lib/runners";
import { formatUsd, useBurst } from "@/lib/burst";
import type { BurstConfig } from "@/gen/BurstConfig";
import type { BurstOverview } from "@/gen/BurstOverview";
import { useState } from "react";
import { Link } from "@tanstack/react-router";
import { Card, SettingsPage } from "./SettingsView";
import { homeStatus } from "@/features/setup/summary";
import type { RunnerStatus } from "@/gen/RunnerStatus";
import { canSelfUpdate, UpdateRunnerButton } from "./RunnerUpdate";
import { networkDiffers, RunnerNetwork } from "./RunnerNetwork";
import { HomeUpdateCard } from "./HomeUpdateCard";
import { CliUpdatesCard } from "./CliUpdates";

export function RunnersView() {
  return <HomeRunners />;
}

/** The command that installs and pairs a runner. A Home in a Space serves an install script with its own build (so both
 * always match), and its gateway gives the command (`install` in `/_home/status`, the same as its setup page);
 * another Home pairs with `launchpad connect`. */
export function useInstallCommand(): string | null {
  const space = delegatedAccess();
  const q = useQuery({ queryKey: ["home-setup-status"], queryFn: homeStatus, enabled: space, staleTime: 60_000 });
  if (!space) return `launchpad connect ${location.origin}`;
  return typeof q.data?.install === "string" ? q.data.install : null;
}

/** A shell command on one line, scrolling sideways, with Copy: people copy it, they do not read it. */
export function CommandLine({ command }: { command: string }) {
  return (
    <div className="flex items-center gap-2">
      <code className="min-w-0 flex-1 overflow-x-auto whitespace-nowrap rounded-control border border-line-2 bg-code px-2.5 py-1.5 font-mono text-[12px]" title={command}>{command}</code>
      <button className="btn btn-sm shrink-0" onClick={() => void copyText(command).then(() => toast("Copied"))}><Icon name="copy" size={12} />Copy</button>
    </div>
  );
}

function Step({ n, children }: { n: number; children: React.ReactNode }) {
  return (
    <li className="grid grid-cols-[20px_minmax(0,1fr)] items-start gap-2 text-[13px] text-ink-2">
      <span className="grid h-5 w-5 place-items-center rounded-full bg-sunk text-[11px] font-semibold text-ink-3">{n}</span>
      <div className="grid min-w-0 grid-cols-[minmax(0,1fr)] gap-1.5 pt-px">{children}</div>
    </li>
  );
}

/** The steps that install and pair a runner, inside the Runners card. */
function AddRunner() {
  const command = useInstallCommand();
  const space = delegatedAccess();
  return (
    <div className="grid gap-2.5">
      <b className="text-[13px] text-ink">Add a runner</b>
      <ol className="m-0 grid list-none gap-2.5 p-0">
        <Step n={1}>{space
          ? <span>On a Linux x86_64 machine (your laptop or a server), sign in to Hugging Face: <code className="font-mono text-[12px]">hf auth login</code> (or set <code className="font-mono text-[12px]">HF_TOKEN</code>).</span>
          : <span>On a Linux machine with Launchpad at Home's build, start <code className="font-mono text-[12px]">launchpad daemon --role runner</code>.</span>}</Step>
        <Step n={2}>
          <span>Run this in a terminal there:</span>
          {command === null ? <Loading what="the install command" /> : <CommandLine command={command} />}
        </Step>
        <Step n={3}><span>It prints a code. Approve the runner here when the same code shows up above.</span></Step>
      </ol>
      <span className="text-[12px] text-ink-3">{space ? "The script checks the machine (bubblewrap, Git), installs Home's build next to any other Launchpad and keeps the runner running as a user service. Run it again to update the runner. On a Mac or an ARM machine, use Burst below." : "The runner keeps its own folders, next to any other Launchpad on that machine. Its own window then shows only its status; everything else stays here."}</span>
    </div>
  );
}

/** How to update a runner whose build differs from Home's: one click for a runner that updates itself, else by hand.
 * A Home in a Space serves its own build through the install command; another Home's runner installs the same
 * Launchpad commit as Home. */
function UpdateRunner({ r }: { r: RunnerStatus }) {
  const command = useInstallCommand();
  const space = delegatedAccess();
  const home = useStatus().data?.commit ?? "";
  const { name, commit } = r.runner;
  const oneClick = canSelfUpdate(r);
  const byHand = space ? (
    <>
      <span>{oneClick ? "Or, in" : "In"} a terminal on {name}, run the install command again. It downloads Home's build, replaces the runner and restarts it; the runner stays paired.</span>
      {command !== null && <CommandLine command={command} />}
    </>
  ) : (
    <span>{oneClick ? "Or, on" : "On"} {name}, install Launchpad at Home's commit <span className="font-mono">{home.slice(0, 12)}</span> (the same way you installed it), then restart its runner daemon (<span className="font-mono">launchpad daemon --role runner</span>). It stays paired.</span>
  );
  return (
    <div className="grid gap-1.5 rounded-card bg-attn-soft px-3 py-2 text-[12px] text-ink-2">
      <span><b className="text-attn">Update {name}.</b> Its build{commit ? ` (${commit.slice(0, 8)})` : ""} differs from Home's ({home.slice(0, 8)}): its running sessions continue, but it takes no new session until it runs Home's build.</span>
      {oneClick ? (
        <>
          <span>Update it from here: {name} downloads Home's build and restarts as soon as none of its agents is in a turn.</span>
          <UpdateRunnerButton runner={r} />
          <details className="text-ink-3"><summary className="cursor-pointer">Update by hand</summary><div className="mt-1 grid gap-1.5">{byHand}</div></details>
        </>
      ) : (
        <>
          {byHand}
          <span className="text-ink-3">The restart interrupts the turns running on {name}: update it when its agents are done. Once updated, it updates with one click next time.</span>
        </>
      )}
    </div>
  );
}

function HomeRunners() {
  const qc = useQueryClient();
  const runners = useRunners();
  const [adding, setAdding] = useState(false);
  const pending = (runners.data ?? []).filter((r) => r.runner.state === "pending");
  const approved = (runners.data ?? []).filter((r) => r.runner.state === "approved");
  const refresh = () => qc.invalidateQueries({ queryKey: ["runners"] });
  // With no runner yet, the steps to add one are the content of the card; afterwards they wait behind Add a runner.
  const showAdd = adding || approved.length === 0;
  return (
    <SettingsPage title="Home and runners" intro="This is Home: it keeps every record and runs no agents. Runners run them: your own Linux machines, and Burst jobs that Home starts when none of them takes a launch.">
      <HomeUpdateCard />
      {runners.isLoading ? <Loading what="runners" /> : runners.error ? <ErrorState error={runners.error} /> : (
        <>
          <Card title={approved.length ? `Runners · ${approved.length}` : "Runners"} tour="settings.runners" end={approved.length > 0 && !adding && <button className="btn btn-sm" onClick={() => setAdding(true)}><Icon name="plus" size={12} />Add a runner</button>}>
            {pending.map((r) => (
              <div key={r.runner.id} className="flex flex-wrap items-center gap-2 rounded-card bg-attn-soft px-3 py-2 text-[13px]">
                <span className="grid min-w-0 flex-1 basis-[240px]">
                  <span className="flex flex-wrap items-center gap-x-2"><b>{r.runner.name}</b><Badge tone="attn">waiting for approval</Badge></span>
                  <span className="text-[12px] text-ink-3">{r.runner.os} · asked {ago(r.runner.created_at)} · approve only if its terminal shows this code</span>
                </span>
                <span className="flex min-w-0 flex-wrap items-center justify-end gap-2">
                <span className="font-mono text-[15px] font-semibold tracking-wide whitespace-nowrap" title="Compare with the code in the terminal">{r.runner.code}</span>
                <ConfirmButton tone="publish" className="btn-primary" consequence={`Lets ${r.runner.name} run agents for this Home. Approve only if the terminal shows ${r.runner.code}.`} confirmLabel="Approve" onConfirm={() => api.post(`/api/runners/${r.runner.id}/approve`, {}).then(() => { toast(`Approved ${r.runner.name}`); setAdding(false); return refresh(); }, toastError)}>Approve</ConfirmButton>
                <MoreMenu label={`More actions for ${r.runner.name}`}>{(close) => <MenuSection><ConfirmButton icon="trash" consequence="Refuses this request." confirmLabel="Refuse" onConfirm={() => api.delete(`/api/runners/${r.runner.id}`).then(() => { close(); return refresh(); }, toastError)}>Refuse</ConfirmButton></MenuSection>}</MoreMenu>
                </span>
              </div>
            ))}
            {approved.length === 0 && pending.length === 0 && <span className="text-[13px] text-ink-3">No runner yet. Until you add one, launches go to Burst (below) when it is on.</span>}
            {approved.map((r) => (
              <div key={r.runner.id} className="grid gap-2">
              <div className="flex flex-wrap items-center gap-2 text-[13px]">
                <span className="grid min-w-0 flex-1">
                  <span className="flex items-center gap-2"><b>{r.runner.name}</b>{r.refused ? <Badge tone="attn" title={r.refused}>refused</Badge> : r.connected ? <Badge tone="ok">connected</Badge> : <Badge>offline</Badge>}{r.runner.capabilities.vm && <Badge tone="teal" title="The virtual machine contains its agents">Local VM</Badge>}{r.runner.capabilities.docker && <Badge tone="teal">Docker</Badge>}{r.compat === "outdated" && <Badge tone="attn" title="Its build differs from Home's: it takes no new sessions. See below how to update it.">Update this runner</Badge>}{networkDiffers(r) && <Badge tone="attn" title="Its last network check found a difference: see below">network differs</Badge>}{r.guard?.paused && <Badge tone="attn" title="Its Takes new sessions switch is off (in its own window): Home starts no new session there">paused</Badge>}</span>
                  <span className="text-[12px] text-ink-3">{r.refused ? `Home refuses it: ${r.refused}` : <>{r.runner.os}{r.runner.last_seen_at ? ` · last connected ${ago(r.runner.last_seen_at)}` : ""}{r.runner.commit ? ` · build ${r.runner.commit.slice(0, 8)}` : ""}</>}</span>
                </span>
                <ConfirmButton icon="trash" consequence={`Disconnects ${r.runner.name}; it must pair again. Its sessions stay on Home.`} confirmLabel="Remove runner" onConfirm={() => api.delete(`/api/runners/${r.runner.id}`).then(() => { toast(`Removed ${r.runner.name}`); return refresh(); }, toastError)}>Remove</ConfirmButton>
              </div>
              {r.compat === "outdated" && <UpdateRunner r={r} />}
              {(r.connected || r.network_check) && <RunnerNetwork r={r} />}
              </div>
            ))}
            {showAdd && (
              <div className={approved.length > 0 || pending.length > 0 ? "grid gap-2.5 border-t border-line pt-3" : "grid gap-2.5"}>
                <AddRunner />
                {adding && approved.length > 0 && <button className="btn btn-ghost btn-sm w-fit" onClick={() => setAdding(false)}>Hide</button>}
              </div>
            )}
          </Card>
          <BurstCard />
          <CliUpdatesCard />
        </>
      )}
      <HomeUpdateCard bottom />
    </SettingsPage>
  );
}

const BURST_STATE: Record<string, string> = { starting: "starting", running: "running", stopping: "stopping", stopped: "stopped", failed: "failed" };

/** Burst: HF Jobs Home starts when no runner takes a launch. On by default, with a monthly cap. */
function BurstCard() {
  const qc = useQueryClient();
  const burst = useBurst();
  const [cap, setCap] = useState<string | null>(null);
  const sessions = useSessions().data ?? [];
  const titleOf = (id: string) => sessions.find((s) => s.id === id)?.title ?? "Session";
  const data = burst.data;
  if (burst.isLoading) return <Card title="Burst"><Loading what="Burst" /></Card>;
  if (burst.error || !data) return <Card title="Burst"><ErrorState error={burst.error} /></Card>;
  const save = (c: BurstConfig) => api.put<BurstOverview>("/api/burst/config", c).then((o) => { qc.setQueryData(["burst"], o); setCap(null); }, toastError);
  const capValue = cap ?? String(data.config.monthly_cap_usd);
  const capNumber = Number(capValue);
  const used = data.config.monthly_cap_usd > 0 ? Math.min(100, (100 * data.month_usd) / data.config.monthly_cap_usd) : 100;
  return (
    <Card title="Burst" tour="settings.burst">
      <span className="text-[13px] text-ink-2">When no runner takes a launch (your laptop is off), Home starts it on Burst: an HF Job in your namespace, from Home's own image. The job holds no token: Home adds them. It stops after each turn, once Home has saved everything, and your next message starts a new one.</span>
      <SettingRow label="Burst" hint={data.unavailable && data.config.enabled ? data.unavailable : "Also available on purpose: Runner → Burst in the composer, or Move to → Burst on a session."}>
        <Choices label="Burst">
          <Choice active={data.config.enabled} onClick={() => !data.config.enabled && void save({ ...data.config, enabled: true })}>On</Choice>
          <Choice active={!data.config.enabled} onClick={() => data.config.enabled && void save({ ...data.config, enabled: false })}>Off</Choice>
        </Choices>
      </SettingRow>
      <SettingRow label="Monthly cap" hint={`${formatUsd(data.month_usd)} used this month (UTC). At the cap, Home starts no new job; running ones finish their turn.`}>
        <span className="flex items-center gap-2">
          <span className="text-[13px] text-ink-3">$</span>
          <input className="field-input w-24" aria-label="Monthly cap in USD" inputMode="decimal" value={capValue} onChange={(e) => setCap(e.target.value)} />
          {cap !== null && <button className="btn btn-sm btn-primary" disabled={!(capNumber >= 0)} onClick={() => void save({ ...data.config, monthly_cap_usd: capNumber })}>Save</button>}
          <span className="h-1.5 w-24 overflow-hidden rounded-full bg-sunk" title={`${Math.round(used)}% of the cap`}><span className={`block h-full ${used >= 100 ? "bg-gh-closed" : used >= 80 ? "bg-attn" : "bg-teal"}`} style={{ width: `${used}%` }} /></span>
        </span>
      </SettingRow>
      <SettingRow label="Hardware" hint="HF Jobs hardware of new jobs. cpu-basic costs 0.01 USD per hour.">
        <span className="font-mono text-[12px]">{data.config.flavor}</span>
      </SettingRow>
      {data.runs.length === 0 ? <Empty icon="clock">No Burst job yet.</Empty> : (
        <div className="grid gap-1.5">
          <b className="text-[12px] text-ink-3">Recent jobs</b>
          {data.runs.slice(0, 10).map((r) => (
            <div key={r.runner_id} className="flex flex-wrap items-center gap-2 text-[13px]">
              <Badge tone={r.state === "running" || r.state === "starting" ? "teal" : r.state === "failed" ? "attn" : undefined}>{BURST_STATE[r.state]}</Badge>
              <span className="min-w-0 flex-1 truncate" title={r.error ?? undefined}>
                {r.session_id ? <Link className="hover:underline" to="/agents" search={{ session: r.session_id }}>{titleOf(r.session_id)}</Link> : "No session"}
                <span className="text-ink-3"> · started {ago(r.created_at)} · {formatUsd(r.cost_per_minute * Math.max(0, ((r.stopped_at ?? Date.now()) - r.created_at) / 60000))}{r.error ? ` · ${r.error}` : ""}</span>
              </span>
              {(r.state === "starting" || r.state === "running" || r.state === "stopping") && <ConfirmButton icon="stop" consequence="Cancels this HF Job now. A turn still running there is interrupted; its session stays on Home." confirmLabel="Stop job" onConfirm={() => api.post<BurstOverview>(`/api/burst/${r.runner_id}/stop`, {}).then((o) => qc.setQueryData(["burst"], o), toastError)}>Stop</ConfirmButton>}
            </div>
          ))}
        </div>
      )}
    </Card>
  );
}
