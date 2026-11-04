import { useQuery } from "@tanstack/react-query";
import { api } from "./api";
import { useScope } from "./workspace";

/** Knowledge base changes by agents since your last look: this window's workspace, or every workspace. */
export function useKnowledgeUnseen(): number {
  const scope = useScope();
  const q = useQuery({ queryKey: ["knowledge-unseen"], queryFn: () => api.get<Record<string, number>>("/api/knowledge/unseen"), staleTime: 60_000, retry: false });
  const counts = q.data ?? {};
  return scope.workspace ? counts[scope.workspace.id] ?? 0 : Object.values(counts).reduce((a, b) => a + b, 0);
}
