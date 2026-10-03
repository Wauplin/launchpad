// Query hooks for daemon data. WebSocket events keep them current (see ws.ts).

import { useQuery } from "@tanstack/react-query";
import type { Config } from "@/gen/Config";
import type { HarnessDefaults } from "@/gen/HarnessDefaults";
import type { CronAgent } from "@/gen/CronAgent";
import type { HarnessKind } from "@/gen/HarnessKind";
import type { Project } from "@/gen/Project";
import type { ScheduledLaunch } from "@/gen/ScheduledLaunch";
import type { Session } from "@/gen/Session";
import type { TerminalOption } from "@/gen/TerminalOption";
import { api } from "./api";

export const useProjects = () => useQuery({ queryKey: ["projects"], queryFn: () => api.get<Project[]>("/api/projects"), staleTime: Infinity });

export function useProject(id: string | null | undefined) {
  const { data } = useProjects();
  return data?.find((p) => p.id === id) ?? null;
}

export const useSessions = () => useQuery({ queryKey: ["sessions"], queryFn: () => api.get<Session[]>("/api/sessions"), staleTime: Infinity });

export function useSession(id: string | null | undefined) {
  const { data } = useSessions();
  return data?.find((s) => s.id === id) ?? null;
}

export const useConfig = () =>
  useQuery({ queryKey: ["config"], queryFn: () => api.get<{ config: Config; error: string | null }>("/api/config"), staleTime: Infinity });

export const useHarnessDefaults = (h: HarnessKind | null) =>
  useQuery({
    queryKey: ["harness-defaults", h],
    queryFn: () => api.get<HarnessDefaults>(`/api/harness/${h}/defaults`),
    enabled: !!h,
    staleTime: 60_000,
  });

export const useTerminals = () => useQuery({ queryKey: ["terminals"], queryFn: () => api.get<TerminalOption[]>("/api/settings/terminals") });

export function projectName(projects: Project[] | undefined, id: string | null, cwd?: string) {
  return projects?.find((p) => p.id === id)?.name ?? cwd?.split("/").pop() ?? "";
}

export const useCrons = () => useQuery({ queryKey: ["crons"], queryFn: () => api.get<CronAgent[]>("/api/launch/crons"), staleTime: Infinity });

export const useSchedules = () => useQuery({ queryKey: ["schedules"], queryFn: () => api.get<ScheduledLaunch[]>("/api/launch/schedules"), staleTime: Infinity });
