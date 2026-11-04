// The sidebar's "Finish setup · N left": a small cached count (`GET /api/setup/checklist/summary`). Kept apart
// from the checklist so the shell does not load it.

import { useQuery } from "@tanstack/react-query";
import type { ChecklistSummary } from "@/gen/ChecklistSummary";
import { api } from "@/lib/api";
import { isDemo } from "@/demo/mode";
import { delegatedAccess } from "@/lib/deployment";

export const SUMMARY_KEY = ["setup-summary"] as const;

/** Home steps counted by the sidebar: the required ones and a runner. */
export const HOME_STEPS = ["huggingface", "key", "github", "ssh", "identity", "profile", "folders", "runner"] as const;

export type HomeStatus = Record<string, { done: boolean } & Record<string, unknown>>;

/** The onboarding steps of a Home in a Space, from its gateway. */
export async function homeStatus(): Promise<HomeStatus> {
  const r = await fetch("/_home/status", { cache: "no-store" });
  if (!r.ok) throw new Error(`Home setup answered ${r.status}`);
  return (await r.json()) as HomeStatus;
}

export const homeLeft = (s: HomeStatus): ChecklistSummary => ({ required_left: HOME_STEPS.filter((k) => !s[k]?.done).length });

export const useSetupSummary = () =>
  useQuery({
    queryKey: SUMMARY_KEY,
    queryFn: async () => (delegatedAccess() ? homeLeft(await homeStatus()) : api.get<ChecklistSummary>("/api/setup/checklist/summary")),
    staleTime: 10 * 60_000,
    enabled: !isDemo,
    retry: false,
  });
