// Composer options of Home and runners: the runner a launch goes to (for example a Local VM for Docker) and the
// registry tools granted to the session, on by their defaults and switched on or off for one launch.

import type { LaunchSpec } from "@/gen/LaunchSpec";
import { FilterChip, FilterChips } from "@/components/ui";
import { useVault } from "@/features/settings/CredentialsView";
import { useMcpServers } from "@/features/settings/McpServers";
import { useStatus } from "@/lib/queries";
import { outdatedLabel, useRunners } from "@/lib/runners";
import { Link } from "@tanstack/react-router";
import { canSelfUpdate, UpdateRunnerButton } from "@/features/settings/RunnerUpdate";
import { useBurst } from "@/lib/burst";
import { useBots } from "@/lib/bots";
import { defaultOn, flipLaunch, launchOn, mcpKey, useLayers } from "@/lib/toolDefaults";

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

type LaunchRunner = ReturnType<typeof useLaunchRunners>[number];

/** Connected, same build as Home, accepting launches from other devices and not paused. */
const takesLaunches = (r: LaunchRunner) => r.connected && r.compat === "same" && r.guard?.accept_launches !== false && !r.guard?.paused;

/** A warning before launching when a runner needs an update: the chosen one, or (Automatic) any connected one. */
export function OutdatedRunnerNotice({ spec }: { spec: LaunchSpec }) {
  const runners = useLaunchRunners();
  const burst = useBurst().data;
  const outdated = runners.filter((r) => r.connected && r.compat === "outdated");
  const chosen = spec.runner_id ? outdated.find((r) => r.runner.id === spec.runner_id) : undefined;
  if (spec.runner_id === "burst" || (spec.runner_id && !chosen) || outdated.length === 0) return null;
  const relevant = chosen ? [chosen] : outdated;
  const names = relevant.map((r) => r.runner.name);
  const oneClick = relevant.filter(canSelfUpdate);
  const effect = chosen
    ? "It takes no new session: choose another runner in the options, or update it first."
    : runners.some(takesLaunches)
      ? "It takes no new session: this launch goes to another runner."
      : burst && !burst.unavailable ? "No up-to-date runner takes launches: this one starts on Burst unless you update it first." : "No up-to-date runner takes launches: update it before launching.";
  return (
    <div className="grid gap-1.5 rounded-lg bg-attn-soft px-3 py-2 text-xs text-attn">
      <span>{outdatedLabel(names)}: its build differs from Home's. {effect} <Link to="/settings/runners" className="font-semibold underline">{oneClick.length === relevant.length ? "Details" : "How to update"}</Link></span>
      {oneClick.map((r) => <UpdateRunnerButton key={r.runner.id} runner={r} />)}
    </div>
  );
}

export function RunnerChoice({ spec, set }: { spec: LaunchSpec; set: Set }) {
  const runners = useLaunchRunners();
  const burst = useBurst().data;
  const online = runners.some(takesLaunches);
  const hint = spec.runner_id === "burst"
    ? "An HF Job started by Home for this session: no laptop needed. It stops after each turn, once Home has saved everything; the next message starts a new one."
    : !spec.runner_id
      ? burst?.unavailable ? `The first runner that takes launches. ${burst.unavailable}.` : online ? "The first runner that takes launches, else Burst (an HF Job started by Home)." : "No runner takes launches now: this one starts on Burst, an HF Job started by Home."
      : "A task that needs Docker runs on a Local VM runner: Docker needs root, so it never runs in a jail.";
  return (
    <span className="grid gap-1">
      <select className="field-input w-auto text-xs" aria-label="Runner" value={spec.runner_id ?? ""} onChange={(e) => set((s) => ({ ...s, runner_id: e.target.value || null }))}>
        <option value="">Automatic</option>
        {runners.map((r) => <option key={r.runner.id} value={r.runner.id} disabled={!r.connected || r.compat === "outdated"}>{r.runner.name}{r.runner.capabilities.vm ? " · Local VM" : ""}{r.runner.capabilities.docker ? " · Docker" : ""}{!r.connected ? " · offline" : r.compat === "outdated" ? " · needs an update" : ""}</option>)}
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

/** The granted tools and MCP servers of a launch, with their state: the defaults of its workspace, folder and bot,
 *  then the launch's own clicks. */
export function useLaunchTools(spec: LaunchSpec, workspaceId: string | null, folderId: string | null) {
  const tools = useGrantedTools({ ...spec, workspace_id: workspaceId });
  const mcp = useGrantedMcp({ ...spec, workspace_id: workspaceId });
  const bot = useBots().data?.find((b) => b.id === spec.bot?.id)?.settings.tools;
  const layers = useLayers(workspaceId, folderId, bot);
  const off = spec.tools_off ?? [];
  const on = spec.tools_on ?? [];
  const items = [...tools.map((t) => ({ key: t.id, name: t.name })), ...mcp.map((s) => ({ key: mcpKey(s.id), name: `${s.name} · MCP` }))];
  return {
    items: items.map((i) => ({ ...i, on: launchOn(layers, off, on, i.key), byDefault: defaultOn(layers, i.key) })),
    flip: (key: string) => flipLaunch(layers, off, on, key),
  };
}

export function ToolsChoice({ spec, set, workspaceId, folderId }: { spec: LaunchSpec; set: Set; workspaceId: string | null; folderId: string | null }) {
  const { items, flip } = useLaunchTools(spec, workspaceId, folderId);
  if (!items.length) return null;
  return (
    <span className="grid gap-1">
      <FilterChips label="Tools">
        {items.map((t) => <FilterChip key={t.key} active={t.on} onClick={() => set((s) => ({ ...s, ...flip(t.key) }))}>{t.name}</FilterChip>)}
      </FilterChips>
      <span className="text-[12px] text-ink-3">{items.some((t) => t.on !== t.byDefault) ? "Changed for this launch. " : ""}Defaults come from the workspace, the folder and the bot.</span>
    </span>
  );
}
