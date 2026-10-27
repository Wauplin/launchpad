import { useNavigate } from "@tanstack/react-router";
import { isPhone } from "@/lib/phone";
// Integrated terminals: the list (refreshed by the `terminals` bus topic) and the actions that open one.

import { useQuery, useQueryClient, type QueryClient } from "@tanstack/react-query";
import { useCallback, useMemo } from "react";
import type { TerminalInfo } from "@/gen/TerminalInfo";
import type { TerminalFolders } from "@/gen/TerminalFolders";
import { toast, toastError } from "@/components/Toast";
import { api } from "@/lib/api";
import { useScope } from "@/lib/workspace";
import { useLayout } from "@/stores/layout";

const listTerminals = () => api.get<TerminalInfo[]>("/api/terminals");
export const useTerminals = () => useQuery({ queryKey: ["terminals"], queryFn: listTerminals, staleTime: Infinity });

/** A fresh list, with the current `busy` state of each shell (the cached list only changes on bus events). */
export const refreshTerminals = (qc: QueryClient) => qc.fetchQuery({ queryKey: ["terminals"], queryFn: listTerminals, staleTime: 0 });

/** The terminals of the current workspace, through their project or folder. Setup terminals (no project) show everywhere. */
export function useScopedTerminals(): TerminalInfo[] {
  const scope = useScope();
  const all = useTerminals().data;
  return useMemo(() => (all ?? []).filter((t) => t.setup || scope.hasProject(t.project_id, t.cwd)), [all, scope]);
}

/** The folder menu of a Terminal button (folder, checkout, other worktrees), fetched when needed. */
export const foldersQuery = (cwd: string) => ({ queryKey: ["terminal-folders", cwd], queryFn: () => api.get<TerminalFolders>(`/api/terminals/folders?cwd=${encodeURIComponent(cwd)}`), staleTime: 10_000 });
export const useTerminalFolders = (cwd: string | null) => useQuery({ ...foldersQuery(cwd ?? ""), enabled: !!cwd });

/** Opens a shell or a session TUI, then shows it in the dock. */
export function useOpenTerminal() {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const reveal = useCallback(
    (t: TerminalInfo) => {
      qc.setQueryData<TerminalInfo[]>(["terminals"], (old) => (old?.some((o) => o.id === t.id) ? old : [...(old ?? []), { ...t, notice: null }]));
      useLayout.getState().setDock({ open: true, active: t.id });
      if (t.notice) toast(t.notice);
      // The phone shows terminals on their own page.
      if (isPhone) void navigate({ to: "/terminals" });
      return t;
    },
    [qc, navigate],
  );
  return useMemo(
    () => ({
      shell: (cwd: string, command?: string, language?: string) => api.post<TerminalInfo>("/api/terminals", { cwd, command, language }).then(reveal).catch(toastError),
      /** Terminal button: the default of the folder menu (the checkout when the worktree was merged or removed). */
      folder: async (cwd: string) => {
        const folders = await qc.fetchQuery(foldersQuery(cwd)).catch(() => null);
        if (folders?.notice) toast(folders.notice);
        return api.post<TerminalInfo>("/api/terminals", { cwd: folders?.default ?? cwd }).then(reveal).catch(toastError);
      },
      tui: (sessionId: string) => api.post<TerminalInfo>(`/api/sessions/${sessionId}/terminal`).then(reveal).catch(toastError),
      /** The sign-in (or install) command of a profile, with its environment, in the home folder of this computer or a host. */
      profileSetup: (profileId: string, host: string | null, install = false) => api.post<TerminalInfo>(`/api/profiles/${profileId}/setup`, { host, install }).then(reveal).catch(toastError),
      /** Restart here: the same shell, snippet or setup command (the daemon replaces the exited tab); a TUI goes through its agent. */
      restart: async (t: TerminalInfo) => {
        try {
          if (t.kind === "tui" && t.session_id) {
            const next = await api.post<TerminalInfo>(`/api/sessions/${t.session_id}/terminal`);
            await api.delete(`/api/terminals/${t.id}`).catch(() => {});
            qc.setQueryData<TerminalInfo[]>(["terminals"], (old) => old?.filter((o) => o.id !== t.id));
            reveal(next);
          } else {
            const next = await api.post<TerminalInfo>(`/api/terminals/${t.id}/restart`);
            qc.setQueryData<TerminalInfo[]>(["terminals"], (old) => old?.filter((o) => o.id !== t.id));
            reveal(next);
          }
        } catch (e) { toastError(e); }
      },
    }),
    [reveal, qc],
  );
}

/** Closing stops the process: ask first for agent TUIs and running commands. Idle shells close at once. */
export const closeNeedsConfirm = (t: TerminalInfo) => t.alive && (t.kind !== "shell" || t.busy !== false);

export const terminalStatus = (t: TerminalInfo) => (t.alive ? (t.busy ? "running a command" : t.kind === "shell" && t.busy === false ? "idle" : "running") : `exited${t.exit_code === null ? "" : ` ${t.exit_code}`}`);

/** The last part of a folder path (`host:/a/b` → `b`). */
export const folderName = (path: string) => path.replace(/[\\/]+$/, "").split(/[\\/:]/).pop() || path;
