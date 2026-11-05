// Settings → Home and runners. On Home: the runners (machines that run agents), with Approve for a pairing request,
// Remove, and the command that installs and pairs a new one. On a runner: its link to Home and the runner guard, local settings that never sync to Home.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import type { Guard } from "@/gen/Guard";
import type { RemoteLaunch } from "@/gen/RemoteLaunch";
import { toast, toastError } from "@/components/Toast";
import { Badge, ConfirmButton, Empty, ErrorState, FilterChip, FilterChips, Loading, SettingRow } from "@/components/ui";
import { Icon } from "@/components/Icon";
import { api } from "@/lib/api";
import { copyText } from "@/lib/clipboard";
import { delegatedAccess } from "@/lib/deployment";
import { formatAge } from "@/lib/format";
import { useConfig, useSessions, useStatus } from "@/lib/queries";
import { useRunners } from "@/lib/runners";
import { formatUsd, useBurst } from "@/lib/burst";
import type { BurstConfig } from "@/gen/BurstConfig";
import type { BurstOverview } from "@/gen/BurstOverview";
import { useState } from "react";
import { Link } from "@tanstack/react-router";
import { Card, SettingsPage } from "./SettingsView";
import { homeStatus } from "@/features/setup/summary";
import type { RunnerStatus } from "@/gen/RunnerStatus";
import { canSelfUpdate, UpdateRunnerButton, UpdateThisRunnerButton } from "./RunnerUpdate";

export function RunnersView() {
  const status = useStatus().data;
  if (!status) return <SettingsPage title="Home and runners"><Loading what="status" /></SettingsPage>;
  if (status.role === "runner") return <ThisRunner />;
  if (status.role === "home") return <HomeRunners />;
  return (
    <SettingsPage title="Home and runners" intro="This Launchpad runs everything in one process. Start it with launchpad daemon --role home on an always-on machine, then pair runners with launchpad connect.">
      <Empty icon="term">No Home and runners in this mode.</Empty>
    </SettingsPage>
  );
}

/** The command that installs and pairs a runner. A Home in a Space serves an install script with its own build (so both
 * always match), and its gateway gives the command (`install` in `/_home/status`, the same as its setup page);
 * another Home pairs with `launchpad connect`. */
function useInstallCommand(): string | null {
  const space = delegatedAccess();
  const q = useQuery({ queryKey: ["home-setup-status"], queryFn: homeStatus, enabled: space, staleTime: 60_000 });
  if (!space) return `launchpad connect ${location.origin}`;
  return typeof q.data?.install === "string" ? q.data.install : null;
}

function AddRunner() {
  const command = useInstallCommand();
  const space = delegatedAccess();
  return (
    <Card title="Add a runner">
      <span className="text-[13px] text-ink-2">{space ? "On your Linux x86_64 laptop or a Linux server, run this in a terminal. On a Mac or an ARM machine, use Burst." : "On the machine, start launchpad daemon --role runner, then run:"}</span>
      {command === null ? <Loading what="the install command" /> : (
        <div className="flex items-start gap-2">
          <pre className="m-0 min-w-0 flex-1 whitespace-pre-wrap break-all rounded-md border border-line-2 bg-code px-2.5 py-2 font-mono text-[12px]">{command}</pre>
          <button className="btn btn-sm" onClick={() => void copyText(command).then(() => toast("Copied"))}><Icon name="copy" size={12} />Copy</button>
        </div>
      )}
      <span className="text-[12px] text-ink-3">{space ? "It needs your Hugging Face sign-in on that machine: run hf auth login there first (or set HF_TOKEN). The script checks the machine (Linux x86_64, bubblewrap, Git), installs a runner with the same build as Home, next to any other Launchpad, starts it as a user service that keeps running after you log out, then asks for your approval below. Run it again to update the runner." : "It prints a code. Approve the request below when the codes match."}</span>
    </Card>
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
      {command !== null && (
        <div className="flex items-start gap-2">
          <pre className="m-0 min-w-0 flex-1 whitespace-pre-wrap break-all rounded-md border border-line-2 bg-code px-2.5 py-2 font-mono text-[12px]">{command}</pre>
          <button className="btn btn-sm" onClick={() => void copyText(command).then(() => toast("Copied"))}><Icon name="copy" size={12} />Copy</button>
        </div>
      )}
    </>
  ) : (
    <span>{oneClick ? "Or, on" : "On"} {name}, install Launchpad at Home's commit <span className="font-mono">{home.slice(0, 12)}</span> (the same way you installed it), then restart its runner daemon (<span className="font-mono">launchpad daemon --role runner</span>). It stays paired.</span>
  );
  return (
    <div className="grid gap-1.5 rounded-lg bg-attn-soft px-3 py-2 text-[12px] text-ink-2">
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
  const pending = (runners.data ?? []).filter((r) => r.runner.state === "pending");
  const approved = (runners.data ?? []).filter((r) => r.runner.state === "approved");
  const refresh = () => qc.invalidateQueries({ queryKey: ["runners"] });
  return (
    <SettingsPage title="Home and runners" intro="This is Home: it keeps every record and runs no agents. Runners run them: your own Linux machines, and Burst jobs that Home starts when none of them takes a launch.">
      {runners.isLoading ? <Loading what="runners" /> : runners.error ? <ErrorState error={runners.error} /> : (
        <>
          {pending.length > 0 && (
            <Card title={`Waiting for approval · ${pending.length}`}>
              {pending.map((r) => (
                <div key={r.runner.id} className="flex flex-wrap items-center gap-2 text-[13px]">
                  <span className="min-w-0 flex-1"><b>{r.runner.name}</b> <span className="text-ink-3">· {r.runner.os} · asked {formatAge(r.runner.created_at)} ago</span></span>
                  <span className="font-mono text-[15px] font-semibold tracking-wide" title="Compare with the code in the terminal">{r.runner.code}</span>
                  <ConfirmButton tone="publish" consequence={`Lets ${r.runner.name} run agents for this Home. Approve only if the terminal shows ${r.runner.code}.`} confirmLabel="Approve" onConfirm={() => api.post(`/api/runners/${r.runner.id}/approve`, {}).then(() => { toast(`Approved ${r.runner.name}`); return refresh(); }, toastError)}>Approve</ConfirmButton>
                  <ConfirmButton icon="trash" consequence="Refuses this request." confirmLabel="Remove" onConfirm={() => api.delete(`/api/runners/${r.runner.id}`).then(refresh, toastError)}>Remove</ConfirmButton>
                </div>
              ))}
            </Card>
          )}
          <Card title="Runners" tour="settings.runners">
            {approved.length === 0 ? <Empty icon="term">No runner yet. Add one with the command below.</Empty> : approved.map((r) => (
              <div key={r.runner.id} className="grid gap-2">
              <div className="flex flex-wrap items-center gap-2 text-[13px]">
                <span className="grid min-w-0 flex-1">
                  <span className="flex items-center gap-2"><b>{r.runner.name}</b>{r.connected ? <Badge tone="ok">connected</Badge> : <Badge>offline</Badge>}{r.runner.capabilities.vm && <Badge tone="teal" title="The virtual machine contains its agents">Local VM</Badge>}{r.runner.capabilities.docker && <Badge tone="teal">Docker</Badge>}{r.compat === "outdated" && <Badge tone="attn" title="Its build differs from Home's: it takes no new sessions. See below how to update it.">Update this runner</Badge>}{r.guard?.paused && <Badge tone="attn">paused</Badge>}{r.guard && !r.guard.accept_launches && <Badge>own launches only</Badge>}</span>
                  <span className="text-[12px] text-ink-3">{r.runner.os}{r.runner.last_seen_at ? ` · last connected ${formatAge(r.runner.last_seen_at)} ago` : ""}{r.runner.commit ? ` · build ${r.runner.commit.slice(0, 8)}` : ""}</span>
                </span>
                <ConfirmButton icon="trash" consequence={`Disconnects ${r.runner.name}; it must pair again. Its sessions stay on Home.`} confirmLabel="Remove runner" onConfirm={() => api.delete(`/api/runners/${r.runner.id}`).then(() => { toast(`Removed ${r.runner.name}`); return refresh(); }, toastError)}>Remove</ConfirmButton>
              </div>
              {r.compat === "outdated" && <UpdateRunner r={r} />}
              </div>
            ))}
          </Card>
          <AddRunner />
          <BurstCard />
        </>
      )}
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
        <FilterChips label="Burst">
          <FilterChip active={data.config.enabled} onClick={() => !data.config.enabled && void save({ ...data.config, enabled: true })}>On</FilterChip>
          <FilterChip active={!data.config.enabled} onClick={() => data.config.enabled && void save({ ...data.config, enabled: false })}>Off</FilterChip>
        </FilterChips>
      </SettingRow>
      <SettingRow label="Monthly cap" hint={`${formatUsd(data.month_usd)} used this month (UTC). At the cap, Home starts no new job; running ones finish their turn.`}>
        <span className="flex items-center gap-2">
          <span className="text-[13px] text-ink-3">$</span>
          <input className="field-input w-24 text-[13px]" aria-label="Monthly cap in USD" inputMode="decimal" value={capValue} onChange={(e) => setCap(e.target.value)} />
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
                <span className="text-ink-3"> · started {formatAge(r.created_at)} ago · {formatUsd(r.cost_per_minute * Math.max(0, ((r.stopped_at ?? Date.now()) - r.created_at) / 60000))}{r.error ? ` · ${r.error}` : ""}</span>
              </span>
              {(r.state === "starting" || r.state === "running" || r.state === "stopping") && <ConfirmButton icon="stop" consequence="Cancels this HF Job now. A turn still running there is interrupted; its session stays on Home." confirmLabel="Stop job" onConfirm={() => api.post<BurstOverview>(`/api/burst/${r.runner_id}/stop`, {}).then((o) => qc.setQueryData(["burst"], o), toastError)}>Stop</ConfirmButton>}
            </div>
          ))}
        </div>
      )}
    </Card>
  );
}

function ThisRunner() {
  const qc = useQueryClient();
  const status = useStatus().data;
  const home = status?.home;
  const guard = home?.guard;
  const boundary = useConfig().data?.config.runner.boundary ?? "jail";
  const launches = useQuery({ queryKey: ["runner-launches"], queryFn: () => api.get<RemoteLaunch[]>("/api/runner/launches"), refetchInterval: 30_000 });
  const save = (g: Guard) => api.put<Guard>("/api/runner/guard", g).then(() => qc.invalidateQueries({ queryKey: ["status"] }), toastError);
  const state: Record<string, string> = { not_connected: "Not paired", connecting: "Connecting…", connected: "Connected", refused: "Refused", offline: "Home unreachable" };
  return (
    <SettingsPage title="This runner" intro="These settings stay on this machine and never sync to Home. They are conveniences, not a security boundary: runners trust Home.">
      <Card title="Home">
        <SettingRow label="Home" hint={home?.message ?? (home?.state === "not_connected" ? "Run launchpad connect <Home address> in a terminal on this machine." : undefined)}>
          <span className="flex items-center gap-2 text-[13px]">{home?.url && <span className="font-mono text-[12px]">{home.url}</span>}<Badge tone={home?.state === "connected" ? "ok" : "attn"}>{state[home?.state ?? "not_connected"]}</Badge>{home?.compat === "outdated" && <Badge tone="attn">Update this runner</Badge>}</span>
        </SettingRow>
        {home?.compat === "outdated" && (
          <div className="grid gap-1 rounded-lg bg-attn-soft px-3 py-2 text-[12px] text-ink-2">
            <span><b className="text-attn">Update this runner.</b> Its build ({status?.commit.slice(0, 8)}) differs from Home's: its running sessions continue, but Home sends it no new session.</span>
            <UpdateThisRunnerButton />
            <span className="text-ink-3">It downloads Home's build and restarts as soon as none of its agents is in a turn. By hand: with a Home in a Space, run the install command from Home's Settings → Home and runners again here; otherwise install Launchpad at Home's commit, then restart this runner daemon.</span>
          </div>
        )}
      </Card>
      <Card title="Boundary">
        <SettingRow label="Agents run in" hint={boundary === "vm" ? "This runner is a Local VM: the virtual machine contains its agents, which may use Docker." : "A bubblewrap jail: agents see only their session's folders, never your files, keys or logins. Set [runner] boundary = \"vm\" in config.toml only inside a dedicated virtual machine."}>
          <Badge tone="teal">{boundary === "vm" ? "Local VM" : "Jail"}</Badge>
        </SettingRow>
      </Card>
      {guard && (
        <Card title="Runner guard">
          <SettingRow label="Launches from other devices" hint="Phone, schedules and bots. Refuse: only windows on this machine launch here.">
            <FilterChips label="Launches from other devices">
              <FilterChip active={guard.accept_launches} onClick={() => !guard.accept_launches && void save({ ...guard, accept_launches: true })}>Accept</FilterChip>
              <FilterChip active={!guard.accept_launches} onClick={() => guard.accept_launches && void save({ ...guard, accept_launches: false })}>Refuse</FilterChip>
            </FilterChips>
          </SettingRow>
          <SettingRow label="Pause runner" hint="No new session starts here; running sessions continue.">
            <FilterChips label="Pause runner">
              <FilterChip active={!guard.paused} onClick={() => guard.paused && void save({ ...guard, paused: false })}>Running</FilterChip>
              <FilterChip active={guard.paused} onClick={() => !guard.paused && void save({ ...guard, paused: true })}>Paused</FilterChip>
            </FilterChips>
          </SettingRow>
        </Card>
      )}
      <Card title="Recent launches from elsewhere">
        {(launches.data ?? []).length === 0 ? <Empty icon="clock">None yet. Launches from the phone, schedules and bots appear here.</Empty> : (launches.data ?? []).slice(0, 20).map((l) => (
          <div key={l.session_id} className="flex items-center gap-2 text-[13px]"><span className="w-[90px] text-[12px] text-ink-3">{formatAge(l.at)} ago</span><span className="min-w-0 flex-1 truncate">{l.title}</span></div>
        ))}
      </Card>
    </SettingsPage>
  );
}
