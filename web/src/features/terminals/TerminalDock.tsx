// Bottom dock with one tab per integrated terminal of the current workspace (plus setup terminals, which have
// no folder and show everywhere). Hide keeps the processes running. Phones show it as the /terminals page.

import { HostLabel } from "@/components/HostLabel";
import { useQueryClient } from "@tanstack/react-query";
import { useNavigate, useRouter, useRouterState } from "@tanstack/react-router";
import { lazy, memo, Suspense, useEffect, useRef, useState } from "react";
import type { Predraft } from "@/gen/Predraft";
import type { TerminalInfo } from "@/gen/TerminalInfo";
import { Icon, type IconName } from "@/components/Icon";
import { MenuItem, Popover } from "@/components/Popover";
import { toast, toastError } from "@/components/Toast";
import { ConfirmButton, Empty, Loading } from "@/components/ui";
import { api } from "@/lib/api";
import { useProjects, useSessions } from "@/lib/queries";
import { useZoomScale } from "@/lib/zoom";
import { useScope } from "@/lib/workspace";
import { useLayout } from "@/stores/layout";
import { PhoneHeader } from "@/mobile/PhoneHeader";
import { TerminalPanel } from "./TerminalPanel";
import { TerminalMenu } from "./TerminalMenu";
import { closeNeedsConfirm, folderName, refreshTerminals, terminalStatus, useOpenTerminal, useScopedTerminals, useTerminals } from "./lib";

// xterm (and its CSS) loads with the first terminal shown, not with the app. A lazy component keeps
// one identity once resolved, so mounted views are never remounted by it.
const TerminalView = lazy(() => import("./TerminalView").then((m) => ({ default: m.TerminalView })));

/** The folder of what is on screen: Files root, project, Local PR or selected session. */
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
    const id = typeof search.session === "string" ? search.session : selected;
    const s = sessions?.find((s) => s.id === id);
    if (s) return s.cwd;
  }
  return (fallback && !fallback.setup ? fallback.cwd : null) ?? scope.projects[0]?.path ?? null;
}

const kindIcon = (t: TerminalInfo): IconName => (t.kind === "tui" ? "agents" : t.kind === "command" ? "play" : "term");
const kindLabel = (t: TerminalInfo) => (t.kind === "tui" ? "TUI" : t.kind === "command" ? "Command" : "Shell");
/** Tab label: the folder (or the agent / setup title), then a chip for the place. */
const tabName = (t: TerminalInfo) => (t.kind === "tui" || t.setup ? t.title : t.folder || folderName(t.cwd));

const Dot = ({ t }: { t: TerminalInfo }) => (
  <i aria-hidden className={`inline-block size-1.5 flex-none rounded-full ${!t.alive ? (t.exit_code === 0 ? "bg-ink-3" : "bg-gh-closed") : t.busy || t.kind !== "shell" ? "bg-attn" : "bg-gh-open"}`} />
);

const Chip = ({ t }: { t: TerminalInfo }) => (
  <>
    {t.setup && <span className="flex-none rounded border border-line px-1 text-[11px] leading-4 text-ink-3">Setup</span>}
    {t.place && <span className="max-w-[140px] flex-none truncate rounded border border-line px-1 font-mono text-[11px] leading-4 text-ink-2" title={t.branch ? `${t.place} · ${t.branch}` : t.place}>{t.place}</span>}
    <HostLabel path={t.cwd} />
  </>
);

const Tab = memo(function Tab({ t, active, onSelect, onClose }: { t: TerminalInfo; active: boolean; onSelect: (id: string) => void; onClose: (t: TerminalInfo) => void }) {
  return (
    <span role="tab" aria-selected={active} className={`inline-flex h-7 max-w-[320px] flex-none items-center gap-1 rounded-md border pl-2 ${active ? "border-teal bg-paper" : "border-line bg-chrome"}`}>
      <button className="flex min-w-0 items-center gap-1.5" title={`${kindLabel(t)} · ${t.title} · ${t.cwd} · ${terminalStatus(t)}`} onClick={() => onSelect(t.id)}>
        <Icon name={kindIcon(t)} size={12} className="flex-none text-ink-3" />
        <span className="min-w-0 truncate">{tabName(t)}</span>
        <Chip t={t} />
        <Dot t={t} />
      </button>
      <button className="iconbtn size-6!" title={t.alive ? (closeNeedsConfirm(t) ? "Stop the process (asks first)" : "Close this idle shell") : "Remove this exited terminal"} aria-label={`Close ${tabName(t)}`} onClick={() => onClose(t)}>
        <Icon name="x" size={12} />
      </button>
    </span>
  );
});

/** Shown over an exited terminal: its exit code, Restart here and Remove. */
function ExitedBanner({ t, onRemove }: { t: TerminalInfo; onRemove: () => void }) {
  const open = useOpenTerminal();
  const [busy, setBusy] = useState(false);
  return (
    <div className="flex flex-none flex-wrap items-center gap-2 border-b border-line-2 bg-chrome px-2 py-1 text-[12px] text-ink-2" role="status">
      <Icon name={t.exit_code === 0 ? "okcircle" : "alert"} size={13} className={t.exit_code === 0 ? "text-ink-3" : "text-danger-ink"} />
      <span>Process exited{t.exit_code === null ? "" : ` (code ${t.exit_code})`}</span>
      <button className="btn btn-sm" disabled={busy} title={t.kind === "tui" ? "Open the agent's TUI again in this folder" : t.kind === "command" ? "Run the same command again in this folder" : "Open a new shell in this folder"} onClick={() => { setBusy(true); void open.restart(t).finally(() => setBusy(false)); }}><Icon name="refresh" size={12} />Restart here</button>
      <button className="btn btn-sm" onClick={onRemove}><Icon name="x" size={12} />Remove</button>
    </div>
  );
}

/** Dock ⋯ menu: closes every terminal in every workspace and removes their tabs. */
export function CloseAllTerminals({ onDone }: { onDone?: () => void } = {}) {
  const count = (useTerminals().data ?? []).length;
  return (
    <div className="grid gap-1 p-1.5 text-[12px]">
      <span className="text-ink-3">{count === 0 ? "No terminal is open." : count === 1 ? "1 terminal, in every workspace." : `${count} terminals, in every workspace.`}</span>
      <ConfirmButton disabled={!count} icon="x" consequence="Stops every process and removes every tab, in every workspace." confirmLabel="Close all" onConfirm={() => api.post("/api/terminals/close-all").then(onDone).catch(toastError)}>Close all terminals</ConfirmButton>
    </div>
  );
}

/** `fill` (phone shell): the dock is a full page, shown while `fill` is true, without height or Hide controls. */
export function TerminalDock({ fill }: { fill?: boolean } = {}) {
  const phone = fill !== undefined;
  const stored = useLayout((s) => s.dock);
  const dock = phone ? { ...stored, open: fill } : stored;
  const setDock = useLayout((s) => s.setDock);
  const terminals = useScopedTerminals();
  const all = useTerminals();
  const previous = useRef<string[]>([]);
  useEffect(() => {
    // Hide when the last tab is removed, but not just because the workspace changed.
    if (all.data && terminals.length === 0 && previous.current.some((id) => !all.data.some((t) => t.id === id))) {
      setDock({ open: false, active: null });
    }
    previous.current = terminals.map((t) => t.id);
  }, [all.data, terminals, setDock]);
  const open = useOpenTerminal();
  const scale = useZoomScale();
  const qc = useQueryClient();
  const navigate = useNavigate();
  const router = useRouter();
  const active = terminals.find((t) => t.id === dock.active) ?? terminals[terminals.length - 1];
  const folder = useCurrentFolder(active);
  const projects = useProjects().data;
  const folderLabel = folder ? (projects?.find((p) => p.path === folder)?.name ?? folderName(folder)) : null;
  const [confirming, setConfirming] = useState<TerminalInfo | null>(null);
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const modifier = event.ctrlKey !== event.metaKey;
      if (!modifier || event.altKey || event.shiftKey || event.key.toLowerCase() !== "t") return;
      event.preventDefault();
      event.stopPropagation();
      if (event.repeat) return;
      if (folder) void open.folder(folder);
      else {
        setDock({ open: true });
        toast("Add a folder or open an agent before opening a shell");
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
  for (const id of visited.current) if (all.data && !all.data.some((t) => t.id === id)) visited.current.delete(id);
  const dropTab = (id: string) => qc.setQueryData<TerminalInfo[]>(["terminals"], (old) => old?.filter((o) => o.id !== id));
  const remove = (t: TerminalInfo) => void api.delete(`/api/terminals/${t.id}`).then(() => dropTab(t.id)).catch(toastError);
  const stop = (t: TerminalInfo) => {
    setConfirming(null);
    void api.post(`/api/terminals/${t.id}/close`).catch(toastError);
  };
  // Idle shells close at once; agent TUIs and running commands ask first (checked against a fresh list).
  const close = async (t: TerminalInfo) => {
    if (!t.alive) return remove(t);
    const fresh = (await refreshTerminals(qc).catch(() => null))?.find((o) => o.id === t.id) ?? t;
    if (!fresh.alive) return remove(fresh);
    if (closeNeedsConfirm(fresh)) setConfirming(fresh);
    else stop(fresh);
  };
  const select = (id: string) => { setConfirming(null); setDock({ active: id }); };
  const tabs = (
    <div className="flex min-w-0 flex-1 gap-1 overflow-x-auto [scrollbar-width:thin]" role="tablist" aria-label="Terminals">
      {terminals.map((t) => <Tab key={t.id} t={t} active={t.id === active?.id} onSelect={select} onClose={(t) => void close(t)} />)}
    </div>
  );
  const dockMenu = (
    <Popover align="right" up={!phone} viewport={phone} width={280} label="Terminal actions" trigger={({ open: o, toggle }) => (
      <button className="iconbtn" aria-label="Terminal actions" title="More terminal actions" aria-haspopup="dialog" aria-expanded={o} onClick={toggle}><Icon name="more" size={16} /></button>
    )}>
      {(closeMenu) => <CloseAllTerminals onDone={closeMenu} />}
    </Popover>
  );
  const switcher = (
    <Popover align="right" viewport width={320} label="Terminals" trigger={({ open: o, toggle }) => (
      <button className="btn min-h-11" aria-haspopup="dialog" aria-expanded={o} onClick={toggle}>
        {terminals.length === 1 ? "1 terminal" : `${terminals.length} terminals`}<Icon name="chev" size={14} />
      </button>
    )}>
      {(closeMenu) => <>
        {terminals.map((t) => (
          <div key={t.id} className="flex min-h-12 items-center gap-1">
            <button className={`flex min-h-12 min-w-0 flex-1 items-center gap-2 rounded-md px-2 text-left text-[13px] ${t.id === active?.id ? "bg-teal-soft text-teal-ink" : ""}`} onClick={() => { select(t.id); closeMenu(); }}>
              <Icon name={kindIcon(t)} size={14} className="flex-none text-ink-3" />
              <span className="min-w-0 flex-1 truncate">{tabName(t)}</span>
              <Chip t={t} />
              <Dot t={t} />
            </button>
            <button className="iconbtn size-11!" aria-label={`Close ${tabName(t)}`} onClick={() => { closeMenu(); select(t.id); void close(t); }}><Icon name="x" size={14} /></button>
          </div>
        ))}
        {terminals.length > 0 && <div className="my-0.5 border-t border-line-2" />}
        {folder && <MenuItem icon={<Icon name="plus" size={14} />} label={`New shell in ${folderLabel}`} onClick={() => { closeMenu(); void open.folder(folder); }} />}
      </>}
    </Popover>
  );
  return (
    <TerminalPanel phone={phone} open={!!dock.open} scale={scale}>
      {phone ? (
        <PhoneHeader
          title="Terminals"
          onBack={() => { if (window.history.length > 1) router.history.back(); else void navigate({ to: "/now" }); }}
          backLabel="Back"
          primary={<div className="flex items-center gap-1">{switcher}{dockMenu}</div>}
          subtitle={active ? `${kindLabel(active)} · ${active.cwd}` : undefined}
        />
      ) : (
        <div className="flex h-9 flex-none items-center gap-1.5 border-b border-line-2 bg-chrome px-2 text-[12px]">
          {tabs}
          <TerminalMenu cwd={folder} label="New" icon="plus" small up title={folder ? `Open a shell in ${folder}` : "Choose a folder for a new shell"} />
          {dockMenu}
          <button className="btn btn-sm" title="Hide the dock; terminals keep running" onClick={() => setDock({ open: false })}><Icon name="hide" size={14} />Hide</button>
        </div>
      )}
      {confirming && (
        <div className="flex flex-none flex-wrap items-center gap-2 border-b border-line-2 bg-attn-soft px-2 py-1 text-[12px] text-ink" role="alert">
          <span className="min-w-0 flex-1">{confirming.kind === "tui" ? `Stop the agent TUI “${confirming.title}”? The TUI process ends; Re-attach the agent from its page to continue.` : `Stop the command running in ${tabName(confirming)}?`}</span>
          <button className="btn btn-sm btn-danger" autoFocus onClick={() => stop(confirming)}>Stop</button>
          <button className="btn btn-sm btn-ghost" onClick={() => setConfirming(null)}>Cancel</button>
        </div>
      )}
      {dock.open && active && !active.alive && <ExitedBanner key={active.id} t={active} onRemove={() => remove(active)} />}
      {[...visited.current].map((id) => (
        <Suspense key={id} fallback={dock.open && id === active?.id ? <Loading what="terminal" className="flex-1 justify-center" /> : null}>
          <TerminalView id={id} scale={scale} visible={dock.open && id === active?.id} />
        </Suspense>
      ))}
      {!active && all.isPending && <Loading what="terminals" className="flex-1 justify-center" />}
      {!active && !all.isPending && (
        <Empty fill icon="term" action={<TerminalMenu cwd={folder} label={folderLabel ? `New terminal in ${folderLabel}` : "New terminal"} up={!phone} phone={phone} align="left" />}>
          No terminal open in this workspace.
        </Empty>
      )}
    </TerminalPanel>
  );
}

/** Status bar chip: the number of terminals in the workspace; shows or hides the dock. */
export function TerminalsChip() {
  const terminals = useScopedTerminals();
  const dock = useLayout((s) => s.dock);
  const setDock = useLayout((s) => s.setDock);
  return (
    <button className={`inline-flex items-center gap-1 rounded border px-1.5 ${dock.open ? "border-teal text-teal-ink" : "border-line"} hover:bg-paper`} aria-pressed={!!dock.open} title={dock.open ? "Hide the terminal dock; terminals keep running" : "Show the terminal dock"} onClick={() => setDock({ open: !dock.open })}>
      <Icon name="term" size={12} />
      Terminals · {terminals.length}
    </button>
  );
}
