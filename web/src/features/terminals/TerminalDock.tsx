// Bottom dock with one tab per integrated terminal of the current workspace. Hide keeps the processes running.

import { useQueryClient } from "@tanstack/react-query";
import { useRouterState } from "@tanstack/react-router";
import { lazy, Suspense, useEffect, useRef, useState } from "react";
import type { Predraft } from "@/gen/Predraft";
import type { TerminalInfo } from "@/gen/TerminalInfo";
import { Icon } from "@/components/Icon";
import { toast, toastError } from "@/components/Toast";
import { api } from "@/lib/api";
import { useConfig, useProjects, useSessions } from "@/lib/queries";
import { zoomScale } from "@/lib/zoom";
import { useScope } from "@/lib/workspace";
import { useLayout, type DockSize } from "@/stores/layout";
import { terminalStatus, useOpenTerminal, useScopedTerminals, useTerminals } from "./lib";

// xterm (and its CSS) loads with the first terminal shown, not with the app. A lazy component keeps
// one identity once resolved, so mounted views are never remounted by it.
const TerminalView = lazy(() => import("./TerminalView").then((m) => ({ default: m.TerminalView })));

const HEIGHTS: Record<DockSize, number> = { small: 200, medium: 320, large: 500 };

/** The folder of what is on screen: code viewer root, project, pre-draft or selected session. */
function useCurrentFolder(fallback: TerminalInfo | undefined): string | null {
  const location = useRouterState({ select: (s) => s.location });
  const projects = useProjects().data;
  const scope = useScope();
  const sessions = useSessions().data;
  const selected = useLayout((s) => s.selectedSession);
  const qc = useQueryClient();
  const path = location.pathname;
  const search = location.search as Record<string, unknown>;
  let m = path.match(/^\/projects\/([^/]+)\/code$/);
  if (m && typeof search.root === "string") return search.root;
  m = path.match(/^\/projects\/([^/]+)(\/code)?$/);
  if (m) return projects?.find((p) => p.id === m![1])?.path ?? null;
  m = path.match(/^\/predrafts\/([^/]+)$/);
  if (m) return qc.getQueryData<{ predraft: Predraft }>(["predrafts", m[1]])?.predraft.root ?? null;
  if (path === "/agents") {
    const s = sessions?.find((s) => s.id === selected);
    if (s) return s.cwd;
  }
  return fallback?.cwd ?? scope.projects[0]?.path ?? null;
}

export function TerminalDock() {
  const dock = useLayout((s) => s.dock);
  const setDock = useLayout((s) => s.setDock);
  const terminals = useScopedTerminals();
  const all = useTerminals().data;
  const previous = useRef<string[]>([]);
  useEffect(() => {
    // Hide when the last tab is removed, but not just because the workspace changed.
    if (all && terminals.length === 0 && previous.current.some((id) => !all.some((t) => t.id === id))) {
      setDock({ open: false, active: null });
    }
    previous.current = terminals.map((t) => t.id);
  }, [all, terminals, setDock]);
  const open = useOpenTerminal();
  const scale = zoomScale(useConfig().data?.config.zoom ?? 2);
  const qc = useQueryClient();
  const active = terminals.find((t) => t.id === dock.active) ?? terminals[terminals.length - 1];
  const folder = useCurrentFolder(active);
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const modifier = event.ctrlKey !== event.metaKey;
      if (!modifier || event.altKey || event.shiftKey || event.key.toLowerCase() !== "t") return;
      event.preventDefault();
      event.stopPropagation();
      if (event.repeat) return;
      if (folder) void open.shell(folder);
      else {
        setDock({ open: true });
        toast("Add a project or open a session before creating a terminal");
      }
    };
    // Capture before the terminal or a text field translates Ctrl+T; this also works with a hidden dock.
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [folder, open, setDock]);
  // Keep visited emulators alive through tab/workspace switches and Hide. Raw byte replay cannot
  // reconstruct historical resizes or terminal modes once the daemon's rolling buffer is truncated.
  const visited = useRef(new Set<string>());
  if (dock.open && active) visited.current.add(active.id);
  for (const id of visited.current) if (all && !all.some((t) => t.id === id)) visited.current.delete(id);
  const close = (t: TerminalInfo) =>
    void (t.alive ? api.post(`/api/terminals/${t.id}/close`) : api.delete(`/api/terminals/${t.id}`).then(() => qc.setQueryData<TerminalInfo[]>(["terminals"], (old) => old?.filter((o) => o.id !== t.id)))).catch(toastError);
  return (
    <section className="flex flex-none flex-col border-t border-line bg-paper" style={{ height: HEIGHTS[dock.size], display: dock.open ? undefined : "none" }} aria-label="Terminals">
      <div className="flex min-h-[34px] flex-none flex-wrap items-center gap-1 border-b border-line-2 bg-chrome px-2 py-1 text-[12px]">
        <div className="flex min-w-0 flex-1 flex-wrap gap-1" role="tablist">
          {terminals.map((t) => (
            <span key={t.id} role="tab" aria-selected={t.id === active?.id} className={`inline-flex max-w-[260px] items-center gap-1 rounded-md border pl-2 ${t.id === active?.id ? "border-teal bg-paper" : "border-line bg-chrome"}`}>
              <button className="min-w-0 truncate py-0.5 text-left" title={`${t.title} · ${t.cwd}`} onClick={() => setDock({ active: t.id })}>
                <Icon name="term" size={12} className="mr-1 inline text-ink-3" />
                {t.kind === "tui" && <span className="mr-1 text-teal-ink">TUI</span>}
                {t.title}
              </button>
              <span className={`text-[11px] ${t.alive ? "text-gh-open" : "text-ink-3"}`}>{terminalStatus(t)}</span>
              <button className="iconbtn" title={t.alive ? "Close: stop the process" : "Remove this exited terminal"} aria-label={`Close ${t.title}`} onClick={() => close(t)}>
                <Icon name="x" size={12} />
              </button>
            </span>
          ))}
          {terminals.length === 0 && <span className="py-0.5 text-ink-3">No terminal open in this workspace.</span>}
        </div>
        <button className="btn btn-sm" disabled={!folder} title={folder ? `Open a shell in ${folder} (Ctrl+T / Cmd+T)` : "Open a project, a session or the code viewer first"} onClick={() => folder && void open.shell(folder)}>New terminal</button>
        <span className="inline-flex gap-0.5 rounded border border-line p-0.5" aria-label="Dock height">
          {(["small", "medium", "large"] as const).map((size) => (
            <button key={size} className={`btn btn-sm ${dock.size === size ? "bg-select!" : ""}`} aria-pressed={dock.size === size} onClick={() => setDock({ size })}>
              {size === "small" ? "S" : size === "medium" ? "M" : "L"}
            </button>
          ))}
        </span>
        <button className="btn btn-sm" title="Hide the dock; terminals keep running" onClick={() => setDock({ open: false })}>Hide</button>
      </div>
      {[...visited.current].map((id) => (
        <Suspense key={id} fallback={dock.open && id === active?.id ? <div className="grid flex-1 place-items-center text-ink-3" role="status">Loading terminal…</div> : null}>
          <TerminalView id={id} scale={scale} visible={dock.open && id === active?.id} />
        </Suspense>
      ))}
      {!active && <div className="grid flex-1 place-items-center text-ink-3">Open a terminal from a project, a worktree, a session, a pre-draft or the code viewer.</div>}
    </section>
  );
}

/** Status bar button: count of open terminals in the workspace, toggles the dock. */
export function TerminalButton() {
  const terminals = useScopedTerminals();
  const dock = useLayout((s) => s.dock);
  const setDock = useLayout((s) => s.setDock);
  const alive = terminals.filter((t) => t.alive).length;
  return (
    <button className={`inline-flex items-center gap-1 rounded border px-1.5 ${dock.open ? "border-teal text-teal-ink" : "border-line"} hover:bg-paper`} title={dock.open ? "Hide the terminal dock" : "Show the terminal dock"} onClick={() => setDock({ open: !dock.open })}>
      <Icon name="term" size={12} />
      Terminal{alive ? ` · ${alive}` : ""}
    </button>
  );
}

/** Settings: closes every terminal, in every workspace (needed before an update installs). */
export function CloseAllTerminals() {
  const alive = (useTerminals().data ?? []).filter((t) => t.alive).length;
  const [busy, setBusy] = useState(false);
  const [asking, setAsking] = useState(false);
  return (
    <div className="flex flex-wrap items-center gap-2 text-[12.5px]">
      <span className="text-ink-2">{alive === 0 ? "No terminal is open." : alive === 1 ? "1 terminal is open." : `${alive} terminals are open.`}</span>
      {asking && alive ? <>
        <button className="btn btn-sm btn-primary" disabled={busy} onClick={() => {
          setBusy(true);
          void api.post("/api/terminals/close-all").catch(toastError).finally(() => { setBusy(false); setAsking(false); });
        }}>Stop {alive === 1 ? "its process" : `all ${alive} processes`}</button>
        <button className="btn btn-sm" disabled={busy} onClick={() => setAsking(false)}>Cancel</button>
      </> : <button className="btn btn-sm" disabled={!alive} onClick={() => setAsking(true)}>Close all terminals</button>}
    </div>
  );
}
