// Phone shell: a top bar with the workspace sheet (workspaces with their counts, nothing else), the section title and
// a status that warns (a detail page replaces the bar with its ‹, title and ⋯: `PhoneBar`, and keeps the status only
// while it warns), one page at a time, and bottom tabs (Today · Chats · Agents · Inbox · More). Chats lists the
// workspace's bots (Ask Launchpad pinned first); More holds the other places. Android back follows `./back.ts`: tab
// taps replace the entries above Today, overlays and detail views own one entry each. Same daemon, rules and feature
// components as the laptop shell; only the frame differs. Every action stays a visible button.

import { recordingZoom } from "@/lib/zoom";
import { lazy, Suspense } from "react";
import { useAddFolder } from "@/features/projects/addFolderStore";
import { Link, Outlet, useRouter, useRouterState } from "@tanstack/react-router";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
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
import { botState, botThread, useBots } from "@/lib/bots";
import { useTodayHere } from "@/features/today/data";
import { isRunning } from "@/lib/sessionStatus";
import { ServerStatus } from "./ServerStatus";
import { MORE_PLACES, PhoneBarContext, pageTitle } from "./PhoneHeader";
import { WorkspaceSheet } from "./WorkspaceSheet";
import { useLaunchIntents } from "./launchIntents";
import { goTab, installBack } from "./back";

const AddFolderDialog = lazy(() => import("@/features/projects/AddFolder").then((m) => ({ default: m.AddFolderDialog })));

/** A bottom tab. `active` overrides the router's match (More and Chats cover several places). A tap replaces the
 * entries above Today (`goTab`), so back from any tab returns to Today; `onAgain` runs on a tap of the current tab. */
function Tab({ to, icon, label, count, highlight, exact = false, active, tour, onAgain }: { to: string; icon: IconName; label: string; count?: number; highlight?: boolean; exact?: boolean; active?: boolean; tour?: string; onAgain?: () => boolean }) {
  const on = "!text-teal-ink font-semibold";
  return (
    <Link
      to={to}
      data-tour={tour}
      onClick={(e) => { if (e.defaultPrevented || e.metaKey || e.ctrlKey || e.shiftKey) return; if (onAgain?.() || goTab(to)) e.preventDefault(); }}
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

/** A text field has the focus: the keyboard is up, so the tab bar hides (the viewport meta resizes the layout). */
function useTyping() {
  const [typing, setTyping] = useState(false);
  useEffect(() => {
    const field = (t: EventTarget | null) => t instanceof HTMLElement && (t.isContentEditable || t.tagName === "TEXTAREA" || (t.tagName === "INPUT" && !["checkbox", "radio", "button", "submit", "range", "file", "color"].includes((t as HTMLInputElement).type)));
    const onIn = (e: FocusEvent) => setTyping(field(e.target));
    const onOut = (e: FocusEvent) => { if (!field(e.relatedTarget)) setTyping(false); };
    document.addEventListener("focusin", onIn);
    document.addEventListener("focusout", onOut);
    return () => { document.removeEventListener("focusin", onIn); document.removeEventListener("focusout", onOut); };
  }, []);
  return typing;
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
  const router = useRouter();
  useLayoutEffect(() => installBack(router.history), [router]);
  useLaunchIntents();
  const path = useRouterState({ select: (s) => s.location.pathname });
  const typing = useTyping();
  const [menu, setMenu] = useState(false);
  const scope = useScope();
  const sessions = (useSessions().data ?? []).filter(scope.hasSession);
  const today = useTodayCount();
  const running = sessions.filter(isRunning).length;
  const inbox = useInboxNeeds();
  // What waits in the other workspaces, shown on the button that opens the sheet.
  const counts = useWorkspaceCounts();
  const inMenu = Object.entries(counts).reduce((n, [id, c]) => n + (id === scope.workspace.id ? 0 : c.total), 0);
  // Chats: the bots of the workspace; its count is the bots that need you. A second tap opens the last chat.
  const bots = (useBots().data ?? []).filter((b) => b.workspace_id === scope.workspace.id);
  const chatsNeed = bots.filter((b) => botState(botThread(sessions, b.id)) === "needs").length;
  const onChats = path === "/chats" || path.startsWith("/bots");
  const lastChat = useRef<string | null>(null);
  if (path.startsWith("/bots/")) lastChat.current = path;
  const openLastChat = () => {
    if (path !== "/chats" || !lastChat.current || !bots.some((b) => lastChat.current === `/bots/${b.id}`)) return false;
    router.history.push(lastChat.current);
    return true;
  };
  const onMore = !onChats && MORE_PLACES.some((p) => path === p || path.startsWith(`${p}/`));
  // More's count: what needs you among its places (failed routines).
  const moreNeeds = useTodayHere().items.filter((i) => i.kind === "routine").length;
  const terminals = path.startsWith("/terminals");
  const viewport = useTerminalViewport(terminals);
  const workspaceName = scope.workspace.name;
  // The top bar a detail page fills (`PhoneBar`); while one is mounted, the section bar hides.
  const [barEl, setBarEl] = useState<HTMLElement | null>(null);
  const [endEl, setEndEl] = useState<HTMLElement | null>(null);
  const [claims, setClaims] = useState(0);
  const bar = useMemo(() => ({ el: barEl, end: endEl, claim: () => { setClaims((n) => n + 1); return () => setClaims((n) => n - 1); } }), [barEl, endEl]);
  return (
    <PhoneBarContext.Provider value={bar}>
    <div className="flex h-dvh flex-col bg-paper pt-[env(safe-area-inset-top)]" style={viewport ? { position: "fixed", insetInline: 0, top: viewport.top, height: viewport.height } : recordingZoom() ? { height: `calc(100dvh / ${recordingZoom()})` } : undefined}>
      <IconSprite />
      {/* Equal side columns keep every section title centered on the viewport; a wide right side (a tab page's button)
          takes the room it needs from the workspace name. */}
      {/* A detail page's bar: its ‹, title and ⋯ (portal), then the status button only while something needs a look. */}
      <div className={claims > 0 ? "phone-bar flex min-h-12 flex-none items-center gap-1 border-b border-line bg-chrome py-0.5 pr-1 pl-1" : "hidden"}>
        <div ref={setBarEl} className="flex min-w-0 flex-1 items-center gap-1" />
        {claims > 0 && <ServerStatus warningsOnly />}
      </div>
      <header className={`${claims > 0 ? "hidden" : "grid"} min-h-12 flex-none grid-cols-[minmax(0,1fr)_auto_minmax(max-content,1fr)] items-center gap-2 border-b border-line bg-chrome px-2`}>
        <button className="btn btn-ghost max-w-full min-w-0 justify-self-start px-2!" aria-label={`Workspace: ${workspaceName}. Change workspace${inMenu > 0 ? ` (${inMenu} waiting)` : ""}`} onClick={() => setMenu(true)}>
          <span className="truncate font-semibold text-ink">{workspaceName}</span><Icon name="chev" size={14} className="shrink-0" />
          <Count n={inMenu} needs title={`${inMenu} waiting in Today of other workspaces`} className="shrink-0" />
        </button>
        <b className="max-w-[40vw] min-w-0 truncate text-center text-[15px]">{pageTitle(path)}</b>
        {/* A tab page's primary button and ⋯ (`PhoneBarEnd`), then the status. */}
        <div className="flex min-w-0 items-center justify-end gap-1"><div ref={setEndEl} className="flex min-w-0 items-center gap-1 empty:hidden" />{claims === 0 && <ServerStatus />}</div>
      </header>
      <main className="relative min-h-0 flex-1 overflow-hidden">
        {empty ? <NoWorkspace /> : !terminals && <Outlet />}
        {/* Visited terminals stay mounted while other pages show. */}
        <div className={terminals ? "flex h-full flex-col" : "hidden"}><TerminalDock fill={terminals} />{terminals && <Outlet />}</div>
      </main>
      {!viewport?.keyboard && !typing && <nav className="flex flex-none border-t border-line bg-chrome pb-[env(safe-area-inset-bottom)]" aria-label="Main">
        <Tab to="/" icon="okcircle" label="Today" count={today} highlight exact />
        <Tab to="/chats" icon="spark" label="Chats" count={chatsNeed} highlight active={onChats} tour="phone.chats-tab" onAgain={openLastChat} />
        <Tab to="/agents" icon="agents" label="Agents" count={running} />
        <Tab to="/inbox" icon="inbox" label="Inbox" count={inbox} highlight />
        <Tab to="/more" icon="more" label="More" count={moreNeeds} highlight active={onMore} tour="phone.more-tab" />
      </nav>}
      <Composer />
      {menu && <WorkspaceSheet onClose={() => setMenu(false)} />}
      <Toasts />
      {addOpen && <Suspense fallback={null}><AddFolderDialog /></Suspense>}
    </div>
    </PhoneBarContext.Provider>
  );
}
