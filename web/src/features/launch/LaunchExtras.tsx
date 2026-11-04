// Composer options of Home and runners: the runner a launch goes to (for example a Local VM for Docker) and the
// registry tools granted to the session, which can be switched off for one launch.

import type { LaunchSpec } from "@/gen/LaunchSpec";
import { FilterChip, FilterChips } from "@/components/ui";
import { useVault } from "@/features/settings/CredentialsView";
import { useStatus } from "@/lib/queries";
import { useRunners } from "@/lib/runners";

type Set = (f: (s: LaunchSpec) => LaunchSpec) => void;

/** Approved runners (Home only), for the Runner option. */
export function useLaunchRunners() {
  return (useRunners().data ?? []).filter((r) => r.runner.state === "approved");
}

export function RunnerChoice({ spec, set }: { spec: LaunchSpec; set: Set }) {
  const runners = useLaunchRunners();
  return (
    <span className="grid gap-1">
      <select className="field-input w-auto text-xs" aria-label="Runner" value={spec.runner_id ?? ""} onChange={(e) => set((s) => ({ ...s, runner_id: e.target.value || null }))}>
        <option value="">Automatic</option>
        {runners.map((r) => <option key={r.runner.id} value={r.runner.id} disabled={!r.connected}>{r.runner.name}{r.runner.capabilities.vm ? " · Local VM" : ""}{r.runner.capabilities.docker ? " · Docker" : ""}{r.connected ? "" : " · offline"}</option>)}
      </select>
      <span className="text-[12px] text-ink-3">A task that needs Docker runs on a Local VM runner: Docker needs root, so it never runs in a jail.</span>
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

export function ToolsChoice({ spec, set }: { spec: LaunchSpec; set: Set }) {
  const tools = useGrantedTools(spec);
  if (!tools.length) return null;
  const off = spec.tools_off ?? [];
  return (
    <FilterChips label="Tools">
      {tools.map((t) => (
        <FilterChip key={t.id} active={!off.includes(t.id)} onClick={() => set((s) => ({ ...s, tools_off: off.includes(t.id) ? off.filter((x) => x !== t.id) : [...off, t.id] }))}>{t.name}</FilterChip>
      ))}
    </FilterChips>
  );
}
