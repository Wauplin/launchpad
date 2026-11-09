// Query hooks for daemon data. WebSocket events keep them current (see ws.ts).

import { useQuery, type UseQueryResult } from "@tanstack/react-query";
import { useRef } from "react";
import type { Config } from "@/gen/Config";
import type { Preset } from "@/gen/Preset";
import type { HarnessDefaults } from "@/gen/HarnessDefaults";
import type { CronAgent } from "@/gen/CronAgent";
import type { HarnessKind } from "@/gen/HarnessKind";
import type { Project } from "@/gen/Project";
import type { ScheduledLaunch } from "@/gen/ScheduledLaunch";
import type { Session } from "@/gen/Session";
import { rememberProjectDefaults } from "@/stores/composer";
import { api } from "./api";

export const useProjects = () => useQuery({ queryKey: ["projects"], queryFn: async () => { const projects = await api.get<Project[]>("/api/projects"); rememberProjectDefaults(projects); return projects; }, staleTime: Infinity });

export function useProject(id: string | null | undefined) {
  const { data } = useProjects();
  return data?.find((p) => p.id === id) ?? null;
}

export const useSessions = () => useQuery({
  queryKey: ["sessions"],
  queryFn: () => api.get<Session[]>("/api/sessions"),
  staleTime: Infinity,
});

export function useSession(id: string | null | undefined) {
  const { data } = useSessions();
  return data?.find((s) => s.id === id) ?? null;
}

export type ConfigResponse = { config: Config; error: string | null; remote: boolean };

export const useConfig = () =>
  useQuery({ queryKey: ["config"], queryFn: () => api.get<ConfigResponse>("/api/config"), staleTime: Infinity });

/** Home's shared settings (`GET /api/settings/shared`; a runner forwards it). Presets live there: launches on every runner read Home's. */
export type SharedSettings = { presets: Preset[]; notify: boolean; github_poll_interval_seconds: number; worktrees_keep: number };

export const useSharedSettings = () =>
  useQuery({ queryKey: ["shared-settings"], queryFn: () => api.get<SharedSettings>("/api/settings/shared"), staleTime: 60_000 });

export const useHarnessDefaults = (h: HarnessKind | null) =>
  useQuery({
    queryKey: ["harness-defaults", h],
    queryFn: () => api.get<HarnessDefaults>(`/api/harness/${h}/defaults`),
    enabled: !!h,
    staleTime: 60_000,
  });

export function projectName(projects: Project[] | undefined, id: string | null, cwd?: string) {
  return projects?.find((p) => p.id === id)?.name ?? cwd?.split("/").pop() ?? "";
}

export const useCrons = () => useQuery({ queryKey: ["crons"], queryFn: () => api.get<CronAgent[]>("/api/launch/crons"), staleTime: Infinity });

export const useSchedules = () => useQuery({ queryKey: ["schedules"], queryFn: () => api.get<ScheduledLaunch[]>("/api/launch/schedules"), staleTime: Infinity });

export function useStatus() {
  return useQuery({ queryKey: ["status"], queryFn: () => api.get<import("@/gen/Status").Status>("/api/status"), staleTime: Infinity });
}

/** A polled query's last settled outcome. TanStack Query puts a query that failed and never had data back to pending at
 * each refetch (`isPending` true, `error` null), so state derived from it would blink at every poll; this keeps the last
 * error until the refetch settles. `pending` is true only before the first answer. */
export function useSettled(q: UseQueryResult<unknown>): { pending: boolean; error: unknown } {
  const last = useRef<unknown>(null);
  if (q.isError) last.current = q.error;
  else if (q.isSuccess) last.current = null;
  const error = q.isError ? q.error : q.isPending && q.errorUpdatedAt > 0 ? last.current : null;
  return { pending: q.isPending && error == null, error };
}
