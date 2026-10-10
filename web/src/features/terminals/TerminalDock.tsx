// Bottom dock with one tab per integrated terminal of the current workspace. Hide keeps the processes running.

import { useQueryClient } from "@tanstack/react-query";
import { useRouterState } from "@tanstack/react-router";
import { useRef, useState } from "react";
import type { Predraft } from "@/gen/Predraft";
import type { TerminalInfo } from "@/gen/TerminalInfo";
import { Icon } from "@/components/Icon";
import { toastError } from "@/components/Toast";
import { api } from "@/lib/api";
import { useConfig, useProjects, useSessions } from "@/lib/queries";
import { zoomScale } from "@/lib/zoom";
import { useLayout, type DockSize } from "@/stores/layout";
import { terminalStatus, useOpenTerminal, useScopedTerminals, useTerminals } from "./lib";
import { TerminalView, type TerminalHandle } from "./TerminalView";

const HEIGHTS: Record<DockSize, number> = { small: 200, medium: 320, large: 500 };

/** The folder of what is on screen: code viewer root, project, pre-draft or selected session. */
function useCurrentFolder(fallback: TerminalInfo | undefined): string | null {
  const location = useRouterState({ select: (s) => s.location });
  const projects = useProjects().data;
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
  return fallback?.cwd ?? null;
}

export function TerminalDock() {
  const dock = useLayout((s) => s.dock);
  const setDock = useLayout((s) => s.setDock);
  const terminals = useScopedTerminals();
  const open = useOpenTerminal();
  const scale = zoomScale(useConfig().data?.config.zoom ?? 2);
  const view = useRef<TerminalHandle>(null);
  const qc = useQueryClient();
  const active = terminals.find((t) => t.id === dock.active) ?? terminals[terminals.length - 1];
  const folder = useCurrentFolder(active);
  if (!dock.open) return null;
  const close = (t: TerminalInfo) =>
    void (t.alive ? api.post(`/api/terminals/${t.id}/close`) : api.delete(`/api/terminals/${t.id}`).then(() => qc.setQueryData<TerminalInfo[]>(["terminals"], (old) => old?.filter((o) => o.id !== t.id)))).catch(toastError);
  return (
    <section className="flex flex-none flex-col border-t border-line bg-paper" style={{ height: HEIGHTS[dock.size] }} aria-label="Terminals">
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
        <button className="btn btn-sm" disabled={!folder} title={folder ? `Open a shell in ${folder}` : "Open a project, a session or the code viewer first"} onClick={() => folder && void open.shell(folder)}>New terminal</button>
        <button className="btn btn-sm" disabled={!active} onClick={() => view.current?.copy()}>Copy selection</button>
        <button className="btn btn-sm" disabled={!active?.alive} onClick={() => view.current?.paste()}>Paste</button>
        <button className="btn btn-sm" disabled={!active} onClick={() => view.current?.clear()}>Clear</button>
        <span className="inline-flex gap-0.5 rounded border border-line p-0.5" aria-label="Dock height">
          {(["small", "medium", "large"] as const).map((size) => (
            <button key={size} className={`btn btn-sm ${dock.size === size ? "bg-select!" : ""}`} aria-pressed={dock.size === size} onClick={() => setDock({ size })}>
              {size === "small" ? "S" : size === "medium" ? "M" : "L"}
            </button>
          ))}
        </span>
        <button className="btn btn-sm" title="Hide the dock; terminals keep running" onClick={() => setDock({ open: false })}>Hide</button>
      </div>
      {active ? <TerminalView key={active.id} ref={view} id={active.id} scale={scale} /> : <div className="grid flex-1 place-items-center text-ink-3">Open a terminal from a project, a worktree, a session, a pre-draft or the code viewer.</div>}
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
