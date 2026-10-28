// The sidebar's "Finish setup · N left": a small cached count (`GET /api/setup/checklist/summary`). Kept apart
// from the checklist so the shell does not load it.

import { useQuery } from "@tanstack/react-query";
import type { ChecklistSummary } from "@/gen/ChecklistSummary";
import { api } from "@/lib/api";
import { isDemo } from "@/demo/mode";

export const SUMMARY_KEY = ["setup-summary"] as const;

export const useSetupSummary = () =>
  useQuery({ queryKey: SUMMARY_KEY, queryFn: () => api.get<ChecklistSummary>("/api/setup/checklist/summary"), staleTime: 10 * 60_000, enabled: !isDemo, retry: false });
