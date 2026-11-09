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
import { useStatus } from "@/lib/queries";
import { useRunners } from "@/lib/runners";
import { useBurstRun } from "@/lib/burst";
import { isDemo } from "@/demo/mode";
import { useLayout } from "@/stores/layout";

/** Terminals run on runners and open from Home's window, which relays them; a runner's own window shows its status only. */
export function useTerminalsHere() {
  const role = useStatus().data?.role;
  return isDemo || role === "home";
}

/** Why no terminal can open on this runner now (an agent's runner: offline, a Burst job that is not running, an older build), or null. */
export function useTerminalBlock(runnerId: string | null | undefined): string | null {
  const runners = useRunners().data;
  const run = useBurstRun(runnerId);
  if (isDemo) return null;
  if (!runnerId) return "This agent runs on no runner";
  if (run) return run.state === "running" ? null : "Its Burst job is not running: send a message to continue the agent, then open the terminal";
  const r = runners?.find((x) => x.runner.id === runnerId);
  if (!r) return null;
  if (!r.connected) return `${r.runner.name} is offline: terminals open on the agent's runner once it reconnects`;
  if (!r.runner.capabilities.terminals) return `${r.runner.name} needs an update to open terminals from Home`;
  return null;
}

/** Why no shell can open in a folder now (no runner connected), or null. */
export function useAnyRunnerBlock(): string | null {
  const runners = useRunners().data;
  if (isDemo || !runners) return null;
  return runners.some((r) => r.connected && r.runner.capabilities.terminals) ? null : "No runner is connected: terminals open on a runner";
}

/** `?runner=` for a call Home relays to that runner (it picks one itself without it). */
const onRunner = (runner?: string | null) => (runner ? `runner=${encodeURIComponent(runner)}` : "");

const listTerminals = () => api.get<TerminalInfo[]>("/api/terminals");
export const useTerminals = () => useQuery({ queryKey: ["terminals"], queryFn: listTerminals, staleTime: Infinity });

/** A fresh list, with the current `busy` state of each shell (the cached list only changes on bus events). */
export const refreshTerminals = (qc: QueryClient) => qc.fetchQuery({ queryKey: ["terminals"], queryFn: listTerminals, staleTime: 0 });

/** The terminals of the current workspace, through their project or folder. */
export function useScopedTerminals(): TerminalInfo[] {
  const scope = useScope();
  const all = useTerminals().data;
  return useMemo(() => (all ?? []).filter((t) => scope.hasProject(t.project_id, t.cwd)), [all, scope]);
}

/** The folder menu of a Terminal button (folder, checkout, other worktrees), fetched when needed. */
export const foldersQuery = (cwd: string, runner?: string | null) => ({
  queryKey: ["terminal-folders", cwd, runner ?? null],
  queryFn: () => api.get<TerminalFolders>(`/api/terminals/folders?cwd=${encodeURIComponent(cwd)}${runner ? `&${onRunner(runner)}` : ""}`),
  staleTime: 10_000,
});
export const useTerminalFolders = (cwd: string | null, runner?: string | null) => useQuery({ ...foldersQuery(cwd ?? "", runner), enabled: !!cwd });

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
      /** `runner`: the runner whose path `cwd` is (an agent's folder, a folder menu entry); Home finds it from the path otherwise. */
      shell: (cwd: string, command?: string, language?: string, runner?: string | null) => api.post<TerminalInfo>(`/api/terminals${runner ? `?${onRunner(runner)}` : ""}`, { cwd, command, language }).then(reveal).catch(toastError),
      /** Terminal button: the default of the folder menu (the checkout when the worktree was merged or removed). */
      folder: async (cwd: string, runner?: string | null) => {
        const folders = await qc.fetchQuery(foldersQuery(cwd, runner)).catch(() => null);
        if (folders?.notice) toast(folders.notice);
        const on = folders?.runner_id ?? runner;
        return api.post<TerminalInfo>(`/api/terminals${on ? `?${onRunner(on)}` : ""}`, { cwd: folders?.default ?? cwd }).then(reveal).catch(toastError);
      },
      tui: (sessionId: string) => api.post<TerminalInfo>(`/api/sessions/${sessionId}/terminal`).then(reveal).catch(toastError),
      /** Restart here: the same shell or snippet (the daemon replaces the exited tab); a TUI goes through its agent. */
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

/** The last part of a folder path (`/a/b` → `b`). */
export const folderName = (path: string) => path.replace(/[\\/]+$/, "").split(/[\\/:]/).pop() || path;
