// Settings → Home and runners. On Home: the runners (machines that run agents), with Approve for a pairing request
// and Remove. On a runner: its link to Home and the runner guard, local settings that never sync to Home.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import type { BackupInfo } from "@/gen/BackupInfo";
import type { Guard } from "@/gen/Guard";
import type { RemoteLaunch } from "@/gen/RemoteLaunch";
import { toast, toastError } from "@/components/Toast";
import { Badge, ConfirmButton, Empty, ErrorState, FilterChip, FilterChips, Loading, SettingRow } from "@/components/ui";
import { api } from "@/lib/api";
import { formatAge } from "@/lib/format";
import { useStatus } from "@/lib/queries";
import { useRunners } from "@/lib/runners";
import { Card, SettingsPage } from "./SettingsView";

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

function HomeRunners() {
  const qc = useQueryClient();
  const runners = useRunners();
  const pending = (runners.data ?? []).filter((r) => r.runner.state === "pending");
  const approved = (runners.data ?? []).filter((r) => r.runner.state === "approved");
  const refresh = () => qc.invalidateQueries({ queryKey: ["runners"] });
  const backup = useQuery({ queryKey: ["home-backup"], queryFn: () => api.get<BackupInfo>("/api/home/backup"), refetchInterval: 30_000 });
  return (
    <SettingsPage title="Home and runners" intro="This is Home: it keeps every record and runs no agents. Runners run them. To add one, run launchpad connect <this address> on it and approve the code here.">
      {runners.isLoading ? <Loading what="runners" /> : runners.error ? <ErrorState error={runners.error} /> : (
        <>
          {pending.length > 0 && (
            <Card title={`Waiting for approval · ${pending.length}`}>
              {pending.map((r) => (
                <div key={r.runner.id} className="flex flex-wrap items-center gap-2 text-[13px]">
                  <span className="min-w-0 flex-1"><b>{r.runner.name}</b> <span className="text-ink-3">· {r.runner.os} · asked {formatAge(r.runner.created_at)} ago</span></span>
                  <span className="font-mono text-[15px] font-semibold tracking-wide" title="Compare with the code shown by launchpad connect">{r.runner.code}</span>
                  <ConfirmButton tone="publish" consequence={`Lets ${r.runner.name} run agents for this Home. Approve only if launchpad connect shows ${r.runner.code}.`} confirmLabel="Approve" onConfirm={() => api.post(`/api/runners/${r.runner.id}/approve`, {}).then(() => { toast(`Approved ${r.runner.name}`); return refresh(); }, toastError)}>Approve</ConfirmButton>
                  <ConfirmButton icon="trash" consequence="Refuses this request." confirmLabel="Remove" onConfirm={() => api.delete(`/api/runners/${r.runner.id}`).then(refresh, toastError)}>Remove</ConfirmButton>
                </div>
              ))}
            </Card>
          )}
          <Card title="Backup" end={backup.data?.destination ? <button className="btn btn-sm" onClick={() => void api.post<BackupInfo>("/api/home/backup", {}).then((b) => { qc.setQueryData(["home-backup"], b); toast("Backed up"); }, toastError)}>Back up now</button> : undefined}>
            {!backup.data?.destination ? <span className="text-[13px] text-ink-2">No backup destination. Set <code className="font-mono">[home] backup</code> in config.toml to <code className="font-mono">hf://buckets/&lt;you&gt;/launchpad</code> or a folder.</span> : (
              <span className="flex flex-wrap items-center gap-2 text-[13px]">
                <span className="font-mono text-[12px]">{backup.data.destination}</span>
                {backup.data.last_complete_at ? <Badge tone="ok">last complete backup {formatAge(backup.data.last_complete_at)} ago</Badge> : <Badge tone="attn">no complete backup yet</Badge>}
                {backup.data.error && <span className="text-[12px] text-danger-ink">{backup.data.error}</span>}
              </span>
            )}
          </Card>
          <Card title="Runners">
            {approved.length === 0 ? <Empty icon="term">No runner yet. Run launchpad connect on a machine.</Empty> : approved.map((r) => (
              <div key={r.runner.id} className="flex flex-wrap items-center gap-2 text-[13px]">
                <span className="grid min-w-0 flex-1">
                  <span className="flex items-center gap-2"><b>{r.runner.name}</b>{r.connected ? <Badge tone="ok">connected</Badge> : <Badge>offline</Badge>}{r.compat === "outdated" && <Badge tone="attn">Update this runner</Badge>}{r.guard?.paused && <Badge tone="attn">paused</Badge>}{r.guard && !r.guard.accept_launches && <Badge>own launches only</Badge>}</span>
                  <span className="text-[12px] text-ink-3">{r.runner.os}{r.runner.last_seen_at ? ` · last connected ${formatAge(r.runner.last_seen_at)} ago` : ""}{r.runner.commit ? ` · build ${r.runner.commit.slice(0, 8)}` : ""}</span>
                </span>
                <ConfirmButton icon="trash" consequence={`Disconnects ${r.runner.name}; it must pair again. Its sessions stay on Home.`} confirmLabel="Remove runner" onConfirm={() => api.delete(`/api/runners/${r.runner.id}`).then(() => { toast(`Removed ${r.runner.name}`); return refresh(); }, toastError)}>Remove</ConfirmButton>
              </div>
            ))}
          </Card>
        </>
      )}
    </SettingsPage>
  );
}

function ThisRunner() {
  const qc = useQueryClient();
  const home = useStatus().data?.home;
  const guard = home?.guard;
  const launches = useQuery({ queryKey: ["runner-launches"], queryFn: () => api.get<RemoteLaunch[]>("/api/runner/launches"), refetchInterval: 30_000 });
  const save = (g: Guard) => api.put<Guard>("/api/runner/guard", g).then(() => qc.invalidateQueries({ queryKey: ["status"] }), toastError);
  const state: Record<string, string> = { not_connected: "Not paired", connecting: "Connecting…", connected: "Connected", refused: "Refused", offline: "Home unreachable" };
  return (
    <SettingsPage title="This runner" intro="These settings stay on this machine and never sync to Home. They are conveniences, not a security boundary: runners trust Home.">
      <Card title="Home">
        <SettingRow label="Home" hint={home?.message ?? (home?.state === "not_connected" ? "Run launchpad connect <Home address> in a terminal on this machine." : undefined)}>
          <span className="flex items-center gap-2 text-[13px]">{home?.url && <span className="font-mono text-[12px]">{home.url}</span>}<Badge tone={home?.state === "connected" ? "ok" : "attn"}>{state[home?.state ?? "not_connected"]}</Badge>{home?.compat === "outdated" && <Badge tone="attn">Update this runner</Badge>}</span>
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
