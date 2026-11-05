// Composer options of Home and runners: the runner a launch goes to (for example a Local VM for Docker) and the
// registry tools granted to the session, which can be switched off for one launch.

import type { LaunchSpec } from "@/gen/LaunchSpec";
import { FilterChip, FilterChips } from "@/components/ui";
import { useVault } from "@/features/settings/CredentialsView";
import { useMcpServers } from "@/features/settings/McpServers";
import { useStatus } from "@/lib/queries";
import { useRunners } from "@/lib/runners";
import { useBurst } from "@/lib/burst";

type Set = (f: (s: LaunchSpec) => LaunchSpec) => void;

/** Approved runners (Home only), for the Runner option. */
export function useLaunchRunners() {
  return (useRunners().data ?? []).filter((r) => r.runner.state === "approved");
}

/** The runner a launch goes to, by name (the composer's summary). */
export function runnerName(runners: ReturnType<typeof useLaunchRunners>, id: string | null): string {
  if (id === "burst") return "Burst";
  return runners.find((r) => r.runner.id === id)?.runner.name ?? "a runner";
}

export function RunnerChoice({ spec, set }: { spec: LaunchSpec; set: Set }) {
  const runners = useLaunchRunners();
  const burst = useBurst().data;
  const online = runners.some((r) => r.connected && r.guard?.accept_launches !== false && !r.guard?.paused);
  const hint = spec.runner_id === "burst"
    ? "An HF Job started by Home for this session: no laptop needed. It stops after each turn, once Home has saved everything; the next message starts a new one."
    : !spec.runner_id
      ? burst?.unavailable ? `The first runner that takes launches. ${burst.unavailable}.` : online ? "The first runner that takes launches, else Burst (an HF Job started by Home)." : "No runner takes launches now: this one starts on Burst, an HF Job started by Home."
      : "A task that needs Docker runs on a Local VM runner: Docker needs root, so it never runs in a jail.";
  return (
    <span className="grid gap-1">
      <select className="field-input w-auto text-xs" aria-label="Runner" value={spec.runner_id ?? ""} onChange={(e) => set((s) => ({ ...s, runner_id: e.target.value || null }))}>
        <option value="">Automatic</option>
        {runners.map((r) => <option key={r.runner.id} value={r.runner.id} disabled={!r.connected}>{r.runner.name}{r.runner.capabilities.vm ? " · Local VM" : ""}{r.runner.capabilities.docker ? " · Docker" : ""}{r.connected ? "" : " · offline"}</option>)}
        {burst && <option value="burst" disabled={!!burst.unavailable}>Burst · HF Job{burst.unavailable ? " · unavailable" : ""}</option>}
      </select>
      <span className="text-[12px] text-ink-3">{hint}</span>
    </span>
  );
}

/** Tools of Home's registry granted in this workspace; a click switches one off for this launch. */
export function useGrantedTools(spec: LaunchSpec) {
  const role = useStatus().data?.role;
  const vault = useVault();
  if (role === undefined || !vault.data) return [];
  return vault.data.tools.filter((t) => t.workspaces.includes("*") || (spec.workspace_id !== null && t.workspaces.includes(spec.workspace_id)));
}

/** MCP servers of Home granted in this workspace. */
export function useGrantedMcp(spec: LaunchSpec) {
  const role = useStatus().data?.role;
  const servers = useMcpServers();
  if (role === undefined || !servers.data) return [];
  return servers.data.filter((s) => s.workspaces.includes("*") || (spec.workspace_id !== null && s.workspaces.includes(spec.workspace_id)));
}

export function ToolsChoice({ spec, set }: { spec: LaunchSpec; set: Set }) {
  const tools = useGrantedTools(spec);
  const mcp = useGrantedMcp(spec);
  if (!tools.length && !mcp.length) return null;
  const off = spec.tools_off ?? [];
  const flip = (id: string) => set((s) => ({ ...s, tools_off: off.includes(id) ? off.filter((x) => x !== id) : [...off, id] }));
  return (
    <FilterChips label="Tools">
      {tools.map((t) => <FilterChip key={t.id} active={!off.includes(t.id)} onClick={() => flip(t.id)}>{t.name}</FilterChip>)}
      {mcp.map((s) => <FilterChip key={s.id} active={!off.includes(`mcp:${s.id}`)} onClick={() => flip(`mcp:${s.id}`)}>{s.name} · MCP</FilterChip>)}
    </FilterChips>
  );
}
