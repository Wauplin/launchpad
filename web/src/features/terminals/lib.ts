// Integrated terminals: the list (refreshed by the `terminals` bus topic) and the actions that open one.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useMemo } from "react";
import type { TerminalInfo } from "@/gen/TerminalInfo";
import { toastError } from "@/components/Toast";
import { api } from "@/lib/api";
import { useScope } from "@/lib/workspace";
import { useLayout } from "@/stores/layout";

export const useTerminals = () => useQuery({ queryKey: ["terminals"], queryFn: () => api.get<TerminalInfo[]>("/api/terminals"), staleTime: Infinity });

/** The terminals of the current workspace, through their project or folder. */
export function useScopedTerminals(): TerminalInfo[] {
  const scope = useScope();
  const all = useTerminals().data;
  return useMemo(() => (all ?? []).filter((t) => scope.hasProject(t.project_id, t.cwd)), [all, scope]);
}

/** Opens a shell or a session TUI, then shows it in the dock. */
export function useOpenTerminal() {
  const qc = useQueryClient();
  const reveal = useCallback(
    (t: TerminalInfo) => {
      qc.setQueryData<TerminalInfo[]>(["terminals"], (old) => (old?.some((o) => o.id === t.id) ? old : [...(old ?? []), t]));
      useLayout.getState().setDock({ open: true, active: t.id });
    },
    [qc],
  );
  return useMemo(
    () => ({
      shell: (cwd: string, command?: string, language?: string) => api.post<TerminalInfo>("/api/terminals", { cwd, command, language }).then(reveal).catch(toastError),
      tui: (sessionId: string) => api.post<TerminalInfo>(`/api/sessions/${sessionId}/terminal`).then(reveal).catch(toastError),
    }),
    [reveal],
  );
}

export const terminalStatus = (t: TerminalInfo) => (t.alive ? "running" : `exited${t.exit_code === null ? "" : ` ${t.exit_code}`}`);
