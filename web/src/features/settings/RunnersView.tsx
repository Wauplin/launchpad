// Settings → Home and runners. On Home: the runners (machines that run agents), with Approve for a pairing request
// and Remove. On a runner: its link to Home and the runner guard, local settings that never sync to Home.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import type { Guard } from "@/gen/Guard";
import type { RunnerStatus } from "@/gen/RunnerStatus";
import { toast, toastError } from "@/components/Toast";
import { Badge, ConfirmButton, Empty, ErrorState, FilterChip, FilterChips, Loading, SettingRow } from "@/components/ui";
import { api } from "@/lib/api";
import { formatAge } from "@/lib/format";
import { useStatus } from "@/lib/queries";
import { Card, SettingsPage } from "./SettingsView";

export const useRunners = (enabled: boolean) => useQuery({ queryKey: ["runners"], queryFn: () => api.get<RunnerStatus[]>("/api/runners"), enabled, staleTime: Infinity });

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
  const runners = useRunners(true);
  const pending = (runners.data ?? []).filter((r) => r.runner.state === "pending");
  const approved = (runners.data ?? []).filter((r) => r.runner.state === "approved");
  const refresh = () => qc.invalidateQueries({ queryKey: ["runners"] });
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
    </SettingsPage>
  );
}
