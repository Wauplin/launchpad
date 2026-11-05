// The sidebar's "Finish setup · N left": a small cached count. Kept apart from the checklist so the shell does not
// load it. On Home, it counts the required steps of the one Home setup list (`homeSteps`), never the optional ones.

import { useQuery } from "@tanstack/react-query";
import type { ChecklistSummary } from "@/gen/ChecklistSummary";
import type { RunnerStatus } from "@/gen/RunnerStatus";
import type { VaultOverview } from "@/gen/VaultOverview";
import { api } from "@/lib/api";
import { isDemo } from "@/demo/mode";
import { delegatedAccess } from "@/lib/deployment";
import { useStatus } from "@/lib/queries";

export const SUMMARY_KEY = ["setup-summary"] as const;

/** One step of Home's setup. Only required steps count in "N left"; optional ones never keep Home from being ready. */
export type HomeStep = { id: string; title: string; why: string; required: boolean; tag: string | null; done: boolean; skipped: boolean };

/** The onboarding state of a Home in a Space, from its gateway: `steps` is the list its setup page shows too. */
export type HomeStatus = { steps: HomeStep[]; left: number; ssh?: { public_key?: string | null } } & Record<string, unknown>;

export async function homeStatus(): Promise<HomeStatus> {
  const r = await fetch("/_home/status", { cache: "no-store" });
  if (!r.ok) throw new Error(`Home setup answered ${r.status}`);
  return (await r.json()) as HomeStatus;
}

const left = (steps: HomeStep[]) => steps.filter((s) => s.required && !s.done).length;
export const homeLeft = (s: HomeStatus): ChecklistSummary => ({ required_left: s.left });

export type HomeData = { vault?: VaultOverview; profiles: number; workspaces: number; projects: number; runners?: RunnerStatus[]; bots?: number };

const step = (id: string, title: string, why: string, required: boolean, done: boolean, tag: string | null = null): HomeStep => ({ id, title, why, required, tag, done, skipped: false });

/** The steps of a Home outside a Space (no gateway, so no Hugging Face, key or GitHub steps), from its own data. */
export function homeSteps(d: HomeData): HomeStep[] {
  return [
    step("identity", "Git identity", "The author of your agents' commits.", true, !!(d.vault?.git_name.trim() && d.vault.git_email.trim())),
    step("profile", "Agent profile", "A subscription login and the default model and effort.", true, d.profiles > 0),
    step("folders", "Workspace and first folder", "Folders your agents work in, grouped by workspace.", true, d.workspaces > 0 && d.projects > 0),
    step("runner", "Where agents run", "A Linux machine of yours that runs the agents.", false, d.runners?.some((r) => r.runner.state === "approved") ?? false, "recommended"),
    step("ssh", "SSH key", "Only for pushing over SSH from a runner's own clone: agents push to Home Git, and the GitHub sign-in covers clones.", false, d.vault?.entries.some((e) => e.kind === "ssh_key") ?? false, "optional"),
    step("bot", "First bot", "An agent that stays: one thread, its own instructions, and notes it keeps between sessions.", false, (d.bots ?? 0) > 0, "optional"),
  ];
}

async function localHomeLeft(): Promise<ChecklistSummary> {
  const [vault, config, workspaces, projects] = await Promise.all([
    api.get<VaultOverview>("/api/vault"),
    api.get<{ config: { profiles: unknown[] } }>("/api/config"),
    api.get<unknown[]>("/api/workspaces"),
    api.get<unknown[]>("/api/projects"),
  ]);
  return { required_left: left(homeSteps({ vault, profiles: config.config.profiles.length, workspaces: workspaces.length, projects: projects.length })) };
}

export function useSetupSummary() {
  const role = useStatus().data?.role;
  return useQuery({
    queryKey: SUMMARY_KEY,
    queryFn: async () => (delegatedAccess() ? homeLeft(await homeStatus()) : role === "home" ? localHomeLeft() : api.get<ChecklistSummary>("/api/setup/checklist/summary")),
    staleTime: 10 * 60_000,
    enabled: !isDemo && !!role,
    retry: false,
  });
}
