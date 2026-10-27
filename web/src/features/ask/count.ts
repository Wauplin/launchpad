// Pending Ask Launchpad proposals of this window's workspace, for the sidebar count. Kept small so the
// sidebar does not load the Ask page.

import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { useWorkspace } from "@/stores/workspace";

export type ProposalSummary = { id: string; workspace_id: string | null; state: string };

export const useProposals = () => useQuery({ queryKey: ["proposals"], queryFn: () => api.get<ProposalSummary[]>("/api/proposals"), staleTime: 30_000 });

export function useAskCount(): number {
  const current = useWorkspace((s) => s.current) ?? null;
  return (useProposals().data ?? []).filter((p) => p.state === "proposed" && (p.workspace_id ?? null) === current).length;
}
