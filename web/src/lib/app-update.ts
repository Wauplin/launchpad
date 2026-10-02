import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toastError } from "@/components/Toast";
import { api } from "@/lib/api";

export type UpdateStatus = { state: "idle" | "running" | "complete" | "failed" | "unavailable"; stage: string; message: string; log: string; commit: string; latest_commit: string | null; outdated: boolean };

export function useAppUpdate() {
  const qc = useQueryClient();
  const query = useQuery({ queryKey: ["app-update"], queryFn: () => api.get<UpdateStatus>("/api/settings/update"), refetchInterval: 5000 });
  const start = useMutation({
    mutationFn: () => api.post("/api/settings/update"),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["app-update"] }),
    onError: toastError,
  });
  return { ...query, start, busy: start.isPending || query.data?.state === "running" };
}
