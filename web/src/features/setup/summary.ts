// The sidebar's "Finish setup · N left": a small cached count. Kept apart from the setup page so the shell does not
// load it. It counts the required steps of the one Home setup list (`homeSteps`, or the Space gateway's), never the
// optional ones.

import { useQuery } from "@tanstack/react-query";
import type { BurstOverview } from "@/gen/BurstOverview";
import type { RunnerStatus } from "@/gen/RunnerStatus";
import type { SignInCheck } from "@/gen/SignInCheck";
import type { VaultOverview } from "@/gen/VaultOverview";
import { api } from "@/lib/api";
import { isDemo } from "@/demo/mode";
import { delegatedAccess } from "@/lib/deployment";
import { useStatus } from "@/lib/queries";

export const SUMMARY_KEY = ["setup-summary"] as const;

/** Required setup steps left. */
export type SetupSummary = { required_left: number };

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
export const homeLeft = (s: HomeStatus): SetupSummary => ({ required_left: s.left });

export type HomeData = { vault?: VaultOverview; profiles: { harness: string }[]; workspaces: number; projects: number; runners?: RunnerStatus[]; github?: SignInCheck[]; burst?: BurstOverview };

const step = (id: string, title: string, why: string, required: boolean, done: boolean, tag: string | null = null): HomeStep => ({ id, title, why, required, tag, done, skipped: false });

/** The steps of a Home outside a Space (no gateway, so no Hugging Face or key steps), from its own data. */
export function homeSteps(d: HomeData): HomeStep[] {
  const logins = d.vault?.logins ?? [];
  // A profile whose harness keeps a login in the vault needs one there; other profiles (API keys) need nothing more.
  const hasLogin = (harness: string) => {
    const l = logins.find((x) => x.harness === harness);
    return !l || !!d.vault?.entries.some((e) => e.kind === l.kind);
  };
  const runner = d.runners?.some((r) => r.runner.state === "approved" && !r.runner.burst) ?? false;
  const burst = !!d.burst?.config.enabled && !d.burst.unavailable;
  return [
    step("workspaces", "Workspace", "Groups of folders, for example Work and Side projects. Create at least one; add more any time.", true, d.workspaces > 0),
    step("github", "GitHub", "Home's GitHub login: Clone from GitHub, publication and suggestions use it.", true, !!d.github?.find((c) => c.id === "github")?.signed_in),
    step("profile", "Agent login", "A profile and its login in Home's vault. Agents never use a machine's own sign-in.", true, d.profiles.length > 0 && !!d.vault && d.profiles.every((p) => hasLogin(p.harness))),
    step("identity", "Git identity of agents", "The name and email of your agents' commits, kept in Home's vault.", true, !!(d.vault?.git_name.trim() && d.vault.git_email.trim())),
    step("runner", "Where agents run", "Burst runs them in Hugging Face Jobs, with nothing to install. A runner is a Linux machine of yours.", true, runner || burst),
    step("folders", "First folder", "Clone a repository from GitHub or create a new plain folder, in a workspace.", true, d.workspaces > 0 && d.projects > 0),
  ];
}

async function localHomeLeft(): Promise<SetupSummary> {
  const [vault, config, workspaces, projects, github, runners, burst] = await Promise.all([
    api.get<VaultOverview>("/api/vault"),
    api.get<{ config: { profiles: { harness: string }[] } }>("/api/config"),
    api.get<unknown[]>("/api/workspaces"),
    api.get<unknown[]>("/api/projects"),
    api.get<SignInCheck[]>("/api/setup/sign-in").catch(() => undefined),
    api.get<RunnerStatus[]>("/api/runners").catch(() => undefined),
    api.get<BurstOverview>("/api/burst").catch(() => undefined),
  ]);
  return { required_left: left(homeSteps({ vault, profiles: config.config.profiles, workspaces: workspaces.length, projects: projects.length, github, runners, burst })) };
}

export function useSetupSummary() {
  const role = useStatus().data?.role;
  return useQuery({
    queryKey: SUMMARY_KEY,
    queryFn: async () => (delegatedAccess() ? homeLeft(await homeStatus()) : localHomeLeft()),
    staleTime: 10 * 60_000,
    enabled: !isDemo && role === "home",
    retry: false,
  });
}
