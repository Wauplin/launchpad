// Phone shell (hosting report, section 05): a top bar with the workspace menu and the connection, one page at a
// time, and five bottom tabs (Now · Agents · Ask · Review · Inbox). Same daemon, rules and feature components
// as the desktop shell; only the frame differs. Every action stays a visible button.

import { lazy, Suspense } from "react";
import { useAddFolder } from "@/features/projects/addFolderStore";
import { Link, Outlet, useRouterState } from "@tanstack/react-router";
import { useLayoutEffect, useState, type ReactNode } from "react";
import { IconSprite, Icon, type IconName } from "@/components/Icon";
import { Toasts } from "@/components/Toast";
import { HostPrompts } from "@/app/HostPrompt";
import { Composer } from "@/features/launch/Composer";
import { TerminalDock } from "@/features/terminals/TerminalDock";
import { useInboxNeeds } from "@/features/inbox/data";
import { useReviewCount } from "@/features/review/data";
import { useScope, useWorkspaces, useWorkspaceSync } from "@/lib/workspace";
import { useProjects, useSessions } from "@/lib/queries";
import { isRunning, needsYou } from "@/lib/sessionStatus";
import { setLayoutChoice } from "@/lib/phone";
import { useTheme } from "@/stores/theme";
import { useWorkspace } from "@/stores/workspace";
import { Sheet } from "./Sheet";
import { ServerStatus } from "./ServerStatus";

const AddFolderDialog = lazy(() => import("@/features/projects/AddFolder").then((m) => ({ default: m.AddFolderDialog })));

function pageTitle(path: string): string {
  if (path.startsWith("/now")) return "Now";
  if (path.startsWith("/agents")) return "Agents";
  if (path.startsWith("/review") || path.startsWith("/publish")) return "Review";
  if (path.startsWith("/inbox") || path === "/") return "Inbox";
  if (path.startsWith("/github")) return "GitHub";
  if (path.startsWith("/predrafts")) return "Local PR";
  if (path.startsWith("/ask")) return "Ask";
  if (path.startsWith("/terminals")) return "Terminals";
  if (path.startsWith("/settings")) return "Settings";
  if (path.startsWith("/setup")) return "Setup";
  if (path.startsWith("/projects")) return path.endsWith("/code") ? "Files" : "Folders";
  return "Launchpad";
}

function Tab({ to, icon, label, count, highlight }: { to: string; icon: IconName; label: string; count?: number; highlight?: boolean }) {
  return (
    <Link
      to={to}
      className="relative flex min-h-[54px] flex-1 flex-col items-center justify-center gap-0.5 text-[11px] text-ink-3 no-underline"
      activeProps={{ className: "!text-teal-ink font-semibold" }}
      activeOptions={{ exact: false }}
    >
      <Icon name={icon} size={20} />
      {label}
      {!!count && <span className={`absolute top-1.5 left-[calc(50%+6px)] min-w-[18px] rounded-full px-1 text-center text-[11px] leading-[18px] font-bold ${highlight ? "bg-attn text-white" : "bg-sunk text-ink-2"}`}>{count}</span>}
    </Link>
  );
}

function MenuLink({ to, icon, children, onClick }: { to: string; icon: IconName; children: ReactNode; onClick: () => void }) {
  return <Link to={to} onClick={onClick} className="flex min-h-12 items-center gap-3 border-b border-line-2 px-4 text-[15px] text-ink no-underline"><Icon name={icon} size={18} className="text-ink-3" />{children}</Link>;
}

/** Workspace switch, secondary pages and device settings. */
function WorkspaceMenu({ onClose }: { onClose: () => void }) {
  const { current, switchTo } = useWorkspace();
  const workspaces = useWorkspaces().data ?? [];
  const projects = useProjects().data?.length ?? 0;
  const { theme, toggle } = useTheme();
  const pick = (id: string | null) => { switchTo(id); onClose(); };
  const button = (id: string | null, label: string, detail: string) => (
    <button key={id ?? "all"} className={`flex min-h-12 w-full items-center gap-3 border-b border-line-2 px-4 text-left text-[15px] ${current === id ? "font-semibold text-teal-ink" : ""}`} aria-pressed={current === id} onClick={() => pick(id)}>
      <span className="flex-1 truncate">{label}</span><span className="text-[13px] text-ink-3">{detail}</span>{current === id && <Icon name="check" size={16} />}
    </button>
  );
  return (
    <Sheet title="Workspace" onClose={onClose}>
      <div className="seclabel px-4 pt-3 pb-1">Show in this tab</div>
      {button(null, "Everything", `${projects} folders`)}
      {workspaces.map((w) => button(w.id, w.name, `${w.project_ids.length} folders`))}
      <div className="seclabel px-4 pt-4 pb-1">Go to</div>
      <MenuLink to="/projects" icon="folder" onClick={onClose}>Folders</MenuLink>
      <MenuLink to="/terminals" icon="term" onClick={onClose}>Terminals</MenuLink>
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
  useWorkspaceSync();
  useLayoutEffect(() => {
    document.documentElement.classList.add("lp-phone");
    document.documentElement.style.zoom = "1";
  }, []);
  const path = useRouterState({ select: (s) => s.location.pathname });
  const [menu, setMenu] = useState(false);
  const scope = useScope();
  const sessions = (useSessions().data ?? []).filter(scope.hasSession);
  const needs = sessions.filter(needsYou).length;
  const running = sessions.filter(isRunning).length;
  const publish = useReviewCount();
  const inbox = useInboxNeeds();
  const terminals = path.startsWith("/terminals");
  const viewport = useTerminalViewport(terminals);
  const workspaceName = scope.workspace?.name ?? "Everything";
  return (
    <div className="flex h-dvh flex-col bg-paper pt-[env(safe-area-inset-top)]" style={viewport ? { position: "fixed", insetInline: 0, top: viewport.top, height: viewport.height } : undefined}>
      <IconSprite />
      {/* Equal side columns keep every section title centered on the viewport. */}
      <header className="grid min-h-12 flex-none grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] items-center gap-2 border-b border-line bg-chrome px-2">
        <button className="btn btn-ghost max-w-full min-w-0 justify-self-start px-2!" aria-label={`Workspace: ${workspaceName}. Open the menu`} onClick={() => setMenu(true)}>
          <span className="truncate font-semibold text-ink">{workspaceName}</span><Icon name="chev" size={14} className="shrink-0" />
        </button>
        <b className="max-w-[40vw] min-w-0 truncate text-center text-[15px]">{pageTitle(path)}</b>
        <div className="flex min-w-0 justify-end"><ServerStatus /></div>
      </header>
      <main className="relative min-h-0 flex-1 overflow-hidden">
        {!terminals && <Outlet />}
        {/* Visited terminals stay mounted while other pages show. */}
        <div className={terminals ? "flex h-full flex-col" : "hidden"}><TerminalDock fill={terminals} />{terminals && <Outlet />}</div>
      </main>
      {!viewport?.keyboard && <nav className="flex flex-none border-t border-line bg-chrome pb-[env(safe-area-inset-bottom)]" aria-label="Main">
        <Tab to="/now" icon="alert" label="Now" count={needs} highlight />
        <Tab to="/agents" icon="agents" label="Agents" count={running} />
        <Link to="/ask" className="flex min-h-[54px] flex-1 flex-col items-center justify-center gap-0.5 text-[11px] font-semibold text-teal-ink no-underline" activeProps={{ "aria-current": "page" } as never}>
          <span className="grid size-8 place-items-center rounded-full bg-teal text-white"><Icon name="spark" size={18} /></span><span className="text-[11px]">Ask</span>
        </Link>
        <Tab to="/review" icon="pr" label="Review" count={publish} />
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
