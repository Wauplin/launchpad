// Phone shell: a top bar with the workspace sheet (workspaces with their counts, nothing else), the page title and
// a status that warns, one page at a time, and bottom tabs (Today · Agents · Ask · Inbox · More). Ask opens the
// chat of the workspace's default bot (`askTarget`); More holds the other places. Same daemon, rules and feature
// components as the desktop shell; only the frame differs. Every action stays a visible button.

import { recordingZoom } from "@/lib/zoom";
import { lazy, Suspense } from "react";
import { useAddFolder } from "@/features/projects/addFolderStore";
import { Link, Outlet, useRouterState } from "@tanstack/react-router";
import { useLayoutEffect, useState } from "react";
import { IconSprite, Icon, type IconName } from "@/components/Icon";
import { Count } from "@/components/ui";
import { Toasts } from "@/components/Toast";
import { Composer } from "@/features/launch/Composer";
import { TerminalDock } from "@/features/terminals/TerminalDock";
import { useInboxNeeds } from "@/features/inbox/data";
import { useTodayCount } from "@/features/today/data";
import { useScope, useWorkspaceSync } from "@/lib/workspace";
import { useSessions } from "@/lib/queries";
import { useWorkspaceCounts } from "@/features/workspaces/counts";
import { NoWorkspace } from "@/features/workspaces/WorkspaceMenu";
import { askTarget, useBots } from "@/lib/bots";
import { isRunning } from "@/lib/sessionStatus";
import { ServerStatus } from "./ServerStatus";
import { MORE_PLACES, pageTitle } from "./PhoneHeader";
import { WorkspaceSheet } from "./WorkspaceSheet";

const AddFolderDialog = lazy(() => import("@/features/projects/AddFolder").then((m) => ({ default: m.AddFolderDialog })));

/** A bottom tab. `active` overrides the router's match (More covers several places, Ask one bot page). */
function Tab({ to, params, search, icon, label, count, highlight, exact = false, active, tour }: { to: string; params?: Record<string, string>; search?: Record<string, unknown>; icon: IconName; label: string; count?: number; highlight?: boolean; exact?: boolean; active?: boolean; tour?: string }) {
  const on = "!text-teal-ink font-semibold";
  return (
    <Link
      to={to}
      params={params as never}
      search={search as never}
      data-tour={tour}
      className={`relative flex min-h-[54px] flex-1 flex-col items-center justify-center gap-0.5 text-[11px] text-ink-3 no-underline ${active ? on : ""}`}
      activeProps={active === undefined ? { className: on } : { className: active ? on : "" }}
      aria-current={active ? "page" : undefined}
      activeOptions={{ exact, includeSearch: false }}
    >
      <Icon name={icon} size={20} />
      {label}
      <Count n={count ?? 0} needs={highlight} className="absolute top-1.5 left-[calc(50%+6px)]" />
    </Link>
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
  // Ask: the chat of the workspace's default bot, or the New bot form when the workspace has none.
  const ask = askTarget(useBots().data ?? [], scope.workspace.id);
  const onAsk = !!ask && path === `/bots/${ask.id}`;
  const onMore = !onAsk && MORE_PLACES.some((p) => path === p || path.startsWith(`${p}/`));
  const terminals = path.startsWith("/terminals");
  const viewport = useTerminalViewport(terminals);
  const workspaceName = scope.workspace.name;
  return (
    <div className="flex h-dvh flex-col bg-paper pt-[env(safe-area-inset-top)]" style={viewport ? { position: "fixed", insetInline: 0, top: viewport.top, height: viewport.height } : recordingZoom() ? { height: `calc(100dvh / ${recordingZoom()})` } : undefined}>
      <IconSprite />
      {/* Equal side columns keep every section title centered on the viewport. */}
      <header className="grid min-h-12 flex-none grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] items-center gap-2 border-b border-line bg-chrome px-2">
        <button className="btn btn-ghost max-w-full min-w-0 justify-self-start px-2!" aria-label={`Workspace: ${workspaceName}. Change workspace${inMenu > 0 ? ` (${inMenu} waiting)` : ""}`} onClick={() => setMenu(true)}>
          <span className="truncate font-semibold text-ink">{workspaceName}</span><Icon name="chev" size={14} className="shrink-0" />
          <Count n={inMenu} needs title={`${inMenu} waiting in Today of other workspaces`} className="shrink-0" />
        </button>
        <b className="max-w-[40vw] min-w-0 truncate text-center text-[15px]">{onAsk ? "Ask" : pageTitle(path)}</b>
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
        {ask
          ? <Tab to="/bots/$id" params={{ id: ask.id }} search={{}} icon="spark" label="Ask" active={onAsk} tour="phone.ask" />
          : <Tab to="/bots" search={{ new: true }} icon="spark" label="Ask" active={false} tour="phone.ask" />}
        <Tab to="/inbox" icon="inbox" label="Inbox" count={inbox} highlight />
        <Tab to="/more" icon="more" label="More" active={onMore} tour="phone.more-tab" />
      </nav>}
      <Composer />
      {menu && <WorkspaceSheet onClose={() => setMenu(false)} />}
      <Toasts />
      {addOpen && <Suspense fallback={null}><AddFolderDialog /></Suspense>}
    </div>
  );
}
