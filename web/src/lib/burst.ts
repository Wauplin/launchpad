// Burst on Home: HF Jobs that run a session when no runner takes it. Only Home has them.

import { useQuery } from "@tanstack/react-query";
import type { BurstOverview } from "@/gen/BurstOverview";
import type { BurstRun } from "@/gen/BurstRun";
import { api } from "./api";
import { useStatus } from "./queries";
import { useRunners } from "./runners";

export function useBurst() {
  const home = useStatus().data?.role === "home";
  return useQuery({ queryKey: ["burst"], queryFn: () => api.get<BurstOverview>("/api/burst"), enabled: home, staleTime: Infinity });
}

export function formatUsd(n: number): string {
  return `$${Math.max(0, n).toFixed(n > 0 && n < 0.01 ? 4 : 2)}`;
}

/** The Burst run of a runner ID, when it is one. */
export function useBurstRun(runnerId: string | null | undefined): BurstRun | null {
  const runs = useBurst().data?.runs;
  if (!runnerId || !runs) return null;
  return runs.find((r) => r.runner_id === runnerId) ?? null;
}

export type RunnerTag = { label: string; title: string; tone: "work" | "quiet" | "err" };

/** Where a session runs, when it needs saying: a disconnected runner, or its Burst job's state. */
export function useRunnerTag(runnerId: string | null | undefined): RunnerTag | null {
  const runners = useRunners().data;
  const run = useBurstRun(runnerId);
  if (!runnerId) return null;
  if (run) {
    switch (run.state) {
      case "starting":
        return { label: "Starting Burst…", title: "An HF Job is starting for this session (about 30 seconds).", tone: "work" };
      case "running":
        return { label: "Burst", title: "Runs on an HF Job started by Home.", tone: "quiet" };
      case "stopping":
      case "stopped":
        return { label: "Burst · stopped", title: "The job stopped after the turn, once Home had saved everything. Your next message starts a new one.", tone: "quiet" };
      case "failed":
        return { label: "Burst failed", title: run.error ?? "The Burst job failed.", tone: "err" };
    }
  }
  const r = runners?.find((x) => x.runner.id === runnerId);
  return r && !r.connected ? { label: `${r.runner.name} offline`, title: "Its runner is disconnected: the status is the last one Home received. Your next message continues it on Burst when Burst is on. Archive works now; the runner stops the session when it reconnects.", tone: "quiet" } : null;
}
