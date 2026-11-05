// Runners as Home sees them. Only Home has runners; elsewhere the query stays off.

import { useQuery } from "@tanstack/react-query";
import type { RunnerStatus } from "@/gen/RunnerStatus";
import { api } from "./api";
import { useStatus } from "./queries";

export function useRunners() {
  const home = useStatus().data?.role === "home";
  return useQuery({ queryKey: ["runners"], queryFn: () => api.get<RunnerStatus[]>("/api/runners"), enabled: home, staleTime: Infinity });
}
