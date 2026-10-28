// The onboarding record (`GET/PATCH /api/onboarding`): welcome answer, chapters seen, dismissed items. It is the
// only daemon data the demo mode reads or writes. Small on purpose: the shells import it.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import type { OnboardingState } from "@/gen/OnboardingState";
import { api } from "@/lib/api";

export const useOnboarding = () => useQuery({ queryKey: ["onboarding"], queryFn: () => api.get<OnboardingState>("/api/onboarding"), staleTime: Infinity });

export type OnboardingPatch = { welcome?: "tour" | "skip" | "later"; chapters_seen?: string[]; dismiss?: string[]; restore?: string[] };

export function usePatchOnboarding() {
  const qc = useQueryClient();
  return async (patch: OnboardingPatch) => {
    const next = await api.patch<OnboardingState>("/api/onboarding", patch);
    qc.setQueryData(["onboarding"], next);
    return next;
  };
}
