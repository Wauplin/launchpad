import { useQuery, useQueryClient } from "@tanstack/react-query";
import { runAction } from "@/lib/actions";
import { api } from "@/lib/api";
import { useScope } from "@/lib/workspace";
import type { Topic } from "@/gen/Topic";
export type { Topic };
export function useTopics() {
  const { workspace } = useScope();
  return useQuery({ queryKey: ["topics", workspace.id], queryFn: () => api.get<Topic[]>(`/api/topics?workspace=${encodeURIComponent(workspace.id)}`), enabled: !!workspace.id });
}
export function useTopicMutations() {
  const qc = useQueryClient();
  const refresh = () => qc.invalidateQueries({ queryKey: ["topics"] });
  return {
    refresh,
    add: async (id: string, sessions: string[], present = true) => { await runAction("topic.members", { id }, { session_ids: sessions, present }); await refresh(); },
    create: async (workspace_id: string, name: string) => { const topic = await runAction<Topic>("topic.create", undefined, { workspace_id, name }); await refresh(); return topic; },
  };
}
