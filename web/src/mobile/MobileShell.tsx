// Phone shell (hosting report, section 05): a top bar with the workspace sheet (workspaces with their counts,
// other places) and the connection, one page at a
// time, and bottom tabs (Today · Agents · Bots · Inbox). Same daemon, rules and feature components
// as the desktop shell; only the frame differs. Every action stays a visible button.

import { recordingZoom } from "@/lib/zoom";
import { lazy, Suspense } from "react";
import { useAddFolder } from "@/features/projects/addFolderStore";
import { Link, Outlet, useRouterState } from "@tanstack/react-router";
import { useLayoutEffect, useState, type ReactNode } from "react";
import { IconSprite, Icon, type IconName } from "@/components/Icon";
import { Toasts } from "@/components/Toast";
import { HostPrompts } from "@/app/HostPrompt";
import { Composer } from "@/features/launch/Composer";
import { TerminalDock } from "@/features/terminals/TerminalDock";
import { useTerminalsHere } from "@/features/terminals/lib";
import { useInboxNeeds } from "@/features/inbox/data";
import { useTodayCount } from "@/features/today/data";
import { useScope, useWorkspaces, useWorkspaceSync } from "@/lib/workspace";
import { useSessions } from "@/lib/queries";
import { describeCount, useWorkspaceCounts, workspaceInitial } from "@/features/workspaces/counts";
import { NewWorkspaceForm, NoWorkspace } from "@/features/workspaces/WorkspaceMenu";
import { useKnowledgeUnseen } from "@/lib/knowledge";
import { isRunning } from "@/lib/sessionStatus";
import { setLayoutChoice } from "@/lib/phone";
import { useTheme } from "@/stores/theme";
import { useWorkspace } from "@/stores/workspace";
import { Sheet } from "./Sheet";
import { ServerStatus } from "./ServerStatus";
import { pageTitle } from "./PhoneHeader";

const AddFolderDialog = lazy(() => import("@/features/projects/AddFolder").then((m) => ({ default: m.AddFolderDialog })));

function Tab({ to, icon, label, count, highlight, exact = false }: { to: string; icon: IconName; label: string; count?: number; highlight?: boolean; exact?: boolean }) {
  return (
    <Link
      to={to}
      className="relative flex min-h-[54px] flex-1 flex-col items-center justify-center gap-0.5 text-[11px] text-ink-3 no-underline"
      activeProps={{ className: "!text-teal-ink font-semibold" }}
      activeOptions={{ exact }}
    >
      <Icon name={icon} size={20} />
      {label}
      {!!count && <span className={`absolute top-1.5 left-[calc(50%+6px)] min-w-[18px] rounded-full px-1 text-center text-[11px] leading-[18px] font-bold ${highlight ? "bg-attn text-white" : "bg-sunk text-ink-2"}`}>{count}</span>}
    </Link>
  );
}

/** A place in the menu, with its count like the laptop sidebar (`attn`: something needs you). */
function MenuLink({ to, icon, children, onClick, count, attn }: { to: string; icon: IconName; children: ReactNode; onClick: () => void; count?: number; attn?: boolean }) {
  return <Link to={to} onClick={onClick} className="flex min-h-12 items-center gap-3 border-b border-line-2 px-4 text-[15px] text-ink no-underline">
    <Icon name={icon} size={18} className="text-ink-3" /><span className="min-w-0 flex-1 truncate">{children}</span>
    {!!count && <span className={`min-w-[22px] rounded-full px-1.5 text-center text-[13px] leading-[22px] font-semibold tabular-nums ${attn ? "bg-attn-soft text-attn" : "bg-sunk text-ink-2"}`}>{count}</span>}
  </Link>;
}

/** The workspaces with what waits in each (never their content), secondary pages and device settings. */
function WorkspaceMenu({ onClose }: { onClose: () => void }) {
  const { current, switchTo } = useWorkspace();
  const workspaces = useWorkspaces().data ?? [];
  const counts = useWorkspaceCounts();
  const [creating, setCreating] = useState(false);
  const { theme, toggle } = useTheme();
  const terminalsHere = useTerminalsHere();
  const knowledgeNew = useKnowledgeUnseen();
  const pick = (id: string) => { switchTo(id); onClose(); };
  return (
    <Sheet title="Workspaces" onClose={onClose}>
      <div data-tour="phone.workspaces">
        {workspaces.map((w) => {
          const on = current === w.id;
          const n = on ? 0 : counts[w.id]?.total ?? 0;
          return (
            <button key={w.id} className={`flex min-h-12 w-full items-center gap-3 border-b border-line-2 px-4 text-left text-[15px] ${on ? "font-semibold text-teal-ink" : ""}`} aria-pressed={on} title={on ? undefined : describeCount(counts[w.id])} onClick={() => pick(w.id)}>
              <span className={`grid size-[28px] flex-none place-items-center rounded-[7px] text-[13px] font-semibold ${on ? "bg-teal text-white" : "bg-sunk text-ink-2"}`} aria-hidden>{workspaceInitial(w.name)}</span>
              <span className="flex-1 truncate">{w.name}</span>
              {n > 0 && <span className="min-w-[22px] rounded-full bg-attn-soft px-1.5 text-center text-[13px] leading-[22px] font-semibold text-attn tabular-nums" aria-label={describeCount(counts[w.id])}>{n}</span>}
              {on && <Icon name="check" size={16} />}
            </button>
          );
        })}
        {creating ? <NewWorkspaceForm onDone={onClose} onCancel={() => setCreating(false)} /> : (
          <button className="flex min-h-12 w-full items-center gap-3 border-b border-line-2 px-4 text-left text-[15px] text-teal-ink" onClick={() => setCreating(true)}>
            <span className="grid size-[28px] flex-none place-items-center rounded-[7px] border border-dashed border-line" aria-hidden><Icon name="plus" size={14} /></span>New workspace
          </button>
        )}
      </div>
      <div className="seclabel px-4 pt-4 pb-1">Go to</div>
      <MenuLink to="/schedules" icon="clock" onClick={onClose}>Schedules</MenuLink>
      <MenuLink to="/projects" icon="folder" onClick={onClose}>Folders</MenuLink>
      <MenuLink to="/knowledge" icon="book" onClick={onClose} count={knowledgeNew}>Knowledge</MenuLink>
      {terminalsHere && <MenuLink to="/terminals" icon="term" onClick={onClose}>Terminals</MenuLink>}
      <MenuLink to="/settings" icon="sliders" onClick={onClose}>Settings</MenuLink>
      <div className="seclabel px-4 pt-4 pb-1">This device</div>
      <div className="flex flex-wrap gap-2 px-4 pb-4">
        <button className="btn" aria-pressed={theme === "dark"} onClick={toggle}><Icon name={theme === "dark" ? "moon" : "sun"} size={16} />{theme === "dark" ? "Dark theme" : "Light theme"}</button>
        <button className="btn" title="Use the laptop layout on this device (reloads)" onClick={() => setLayoutChoice("desktop")}>Use desktop layout</button>
      </div>
    </Sheet>
  );
}

/** The visual viewport shrinks for the keyboard even when the layout viewport / dvh does not. */
function useTerminalViewport(enabled: boolean) {
  const [viewport, setViewport] = useState<{ height: number; top: number; keyboard: boolean } | null>(null);
  useLayoutEffect(() => {
    const vv = window.visualViewport;
    if (!enabled || !vv) { setViewport(null); return; }
    const update = () => setViewport({ height: vv.height, top: vv.offsetTop, keyboard: window.innerHeight - vv.height > 120 });
    update();
    vv.addEventListener("resize", update);
    vv.addEventListener("scroll", update);
    window.addEventListener("resize", update);
    return () => { vv.removeEventListener("resize", update); vv.removeEventListener("scroll", update); window.removeEventListener("resize", update); };
  }, [enabled]);
  return enabled ? viewport : null;
}

export function MobileShell() {
  const addOpen = useAddFolder((s) => s.open);
  const { empty } = useWorkspaceSync();
  useLayoutEffect(() => {
    document.documentElement.classList.add("lp-phone");
    document.documentElement.style.zoom = String(recordingZoom() ?? 1);
  }, []);
  const path = useRouterState({ select: (s) => s.location.pathname });
  const [menu, setMenu] = useState(false);
  const scope = useScope();
  const sessions = (useSessions().data ?? []).filter(scope.hasSession);
  const today = useTodayCount();
  const running = sessions.filter(isRunning).length;
  const inbox = useInboxNeeds();
  // What waits in the other workspaces, shown on the button that opens the sheet.
  const counts = useWorkspaceCounts();
  const inMenu = Object.entries(counts).reduce((n, [id, c]) => n + (id === scope.workspace.id ? 0 : c.total), 0);
  const terminals = path.startsWith("/terminals");
  const viewport = useTerminalViewport(terminals);
  const workspaceName = scope.workspace.name;
  return (
    <div className="flex h-dvh flex-col bg-paper pt-[env(safe-area-inset-top)]" style={viewport ? { position: "fixed", insetInline: 0, top: viewport.top, height: viewport.height } : recordingZoom() ? { height: `calc(100dvh / ${recordingZoom()})` } : undefined}>
      <IconSprite />
      {/* Equal side columns keep every section title centered on the viewport. */}
      <header className="grid min-h-12 flex-none grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] items-center gap-2 border-b border-line bg-chrome px-2">
        <button className="btn btn-ghost max-w-full min-w-0 justify-self-start px-2!" aria-label={`Workspace: ${workspaceName}. Open the menu${inMenu > 0 ? ` (${inMenu} waiting)` : ""}`} onClick={() => setMenu(true)}>
          <span className="truncate font-semibold text-ink">{workspaceName}</span><Icon name="chev" size={14} className="shrink-0" />
          {inMenu > 0 && <span className="shrink-0 rounded-full bg-attn-soft px-1.5 text-[11px] leading-[18px] font-semibold text-attn tabular-nums" title={`${inMenu} waiting in Today of other workspaces`}>{inMenu}</span>}
        </button>
        <b className="max-w-[40vw] min-w-0 truncate text-center text-[15px]">{pageTitle(path)}</b>
        <div className="flex min-w-0 justify-end"><ServerStatus /></div>
      </header>
      <main className="relative min-h-0 flex-1 overflow-hidden">
        {empty ? <NoWorkspace /> : !terminals && <Outlet />}
        {/* Visited terminals stay mounted while other pages show. */}
        <div className={terminals ? "flex h-full flex-col" : "hidden"}><TerminalDock fill={terminals} />{terminals && <Outlet />}</div>
      </main>
      {!viewport?.keyboard && <nav className="flex flex-none border-t border-line bg-chrome pb-[env(safe-area-inset-bottom)]" aria-label="Main">
        <Tab to="/" icon="okcircle" label="Today" count={today} highlight exact />
        <Tab to="/agents" icon="agents" label="Agents" count={running} />
        <Link to="/bots" className="flex min-h-[54px] flex-1 flex-col items-center justify-center gap-0.5 text-[11px] font-semibold text-teal-ink no-underline" activeProps={{ "aria-current": "page" } as never}>
          <span className="grid size-8 place-items-center rounded-full bg-teal text-white"><Icon name="spark" size={18} /></span><span className="text-[11px]">Bots</span>
        </Link>
        <Tab to="/inbox" icon="inbox" label="Inbox" count={inbox} highlight />
      </nav>}
      <Composer />
      {menu && <WorkspaceMenu onClose={() => setMenu(false)} />}
      <Toasts />
      <HostPrompts />
      {addOpen && <Suspense fallback={null}><AddFolderDialog /></Suspense>}
    </div>
  );
}
