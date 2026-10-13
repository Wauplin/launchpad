// The list behind To publish and its sidebar count, kept apart so the sidebar does not load the page.
import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import type { Predraft } from "@/gen/Predraft";
import type { Suggestion } from "@/gen/Suggestion";
import { api } from "@/lib/api";
import { useScope } from "@/lib/workspace";

export type Row = { kind: "suggestion"; s: Suggestion; at: number } | { kind: "predraft"; p: Predraft; at: number };

/** Unposted suggestions and unpublished pre-drafts of the workspace, newest first. The sidebar count uses the same list. */
export function usePublishItems(): Row[] {
  const scope = useScope();
  const suggestions = useQuery({ queryKey: ["suggestions"], queryFn: () => api.get<Suggestion[]>("/api/suggestions") }).data;
  const predrafts = useQuery({ queryKey: ["predrafts"], queryFn: () => api.get<Predraft[]>("/api/predrafts") }).data;
  return useMemo(() => {
    const open = (predrafts ?? []).filter((p) => !p.archived && !p.published_url);
    const owner = new Map((predrafts ?? []).map((p) => [p.id, p.project_id]));
    const rows: Row[] = [
      ...(suggestions ?? []).filter((s) => s.status !== "posted" && (s.target.kind === "git_hub" ? scope.hasRepo(s.target.repo) : scope.hasProject(owner.get(s.target.id) ?? null))).map((s) => ({ kind: "suggestion" as const, s, at: s.created_at })),
      ...open.filter((p) => scope.hasProject(p.project_id)).map((p) => ({ kind: "predraft" as const, p, at: p.created_at })),
    ];
    return rows.sort((a, b) => b.at - a.at);
  }, [suggestions, predrafts, scope]);
}
