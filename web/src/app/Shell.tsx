import { Outlet, useRouterState } from "@tanstack/react-router";
import { lazy, Suspense } from "react";
import { useAddFolder } from "@/features/projects/addFolderStore";
import { IconSprite } from "@/components/Icon";
import { Toasts } from "@/components/Toast";
import { Composer } from "@/features/launch/Composer";
import { TerminalDock } from "@/features/terminals/TerminalDock";
import { useLayoutEffect } from "react";
import { useZoomScale } from "@/lib/zoom";
import { useWorkspaceSync } from "@/lib/workspace";
import { Sidebar } from "./Sidebar";
import { inSettings, SettingsSidebar, useRememberPlace } from "./SettingsSidebar";
import { WorkspaceRail } from "@/features/workspaces/Rail";
import { NoWorkspace } from "@/features/workspaces/WorkspaceMenu";
import { StatusBar } from "./StatusBar";
import { TopBar } from "./TopBar";
import { isDemo } from "@/demo/mode";
import { WelcomeGate } from "@/onboarding/WelcomeGate";
import { useAskPanel } from "@/stores/ask";
import { useLaunchIntents } from "@/mobile/launchIntents";

// Demo mode only: the tour overlay and its chapters (src/onboarding/tour/) load with the demo.
const TourRoot = lazy(() => import("@/onboarding/tour/TourRoot").then((m) => ({ default: m.TourRoot })));
// The Ask panel (a bot's chat) loads when Ask opens it.
const AskPanel = lazy(() => import("@/features/chat/AskPanel").then((m) => ({ default: m.AskPanel })));
const AddFolderDialog = lazy(() => import("@/features/projects/AddFolder").then((m) => ({ default: m.AddFolderDialog })));

/** Home is served directly in the Hugging Face Space page, whose mini header (Space name, menu) is a pill fixed 20 px
 * from the top and 24 px from the right, 40 px tall, hidden below 640 px. Its width follows the Space ID, whose
 * length matches the first label of the `*.hf.space` host. Keep the top bar's buttons out of it (see TopBar). Only a
 * page embedded directly by the Hub does it. */
function useHubPillInset() {
  useLayoutEffect(() => {
    const host = location.hostname;
    if (window.parent === window || window.top !== window.parent || !host.endsWith(".hf.space")) return;
    const wide = matchMedia("(min-width: 640px)");
    const root = document.documentElement.style;
    const apply = () => {
      const on = wide.matches;
      root.setProperty("--lp-host-inset-right", on ? `${Math.round(36 + 150 + 9.5 * host.split(".")[0].length)}px` : "0px");
      root.setProperty("--lp-host-inset-top", on ? "64px" : "0px");
    };
    apply();
    wide.addEventListener("change", apply);
    return () => wide.removeEventListener("change", apply);
  }, []);
}

export function Shell() {
  const addOpen = useAddFolder((s) => s.open);
  const askOpen = useAskPanel((s) => s.open);
  const scale = useZoomScale();
  useLayoutEffect(() => {
    document.documentElement.style.zoom = String(scale);
    // Embedding hosts express insets in unzoomed pixels (see TopBar).
    document.documentElement.style.setProperty("--lp-zoom", String(scale));
  }, [scale]);
  const { empty } = useWorkspaceSync();
  useHubPillInset();
  useRememberPlace();
  useLaunchIntents();
  // Settings replaces the sidebar with its own pages, so there is one level of navigation.
  const settings = useRouterState({ select: (s) => inSettings(s.location.pathname) });
  // There is no default workspace: Setup (whose Workspaces step creates the first one) and Settings open without one.
  const setup = useRouterState({ select: (s) => s.location.pathname === "/setup" });
  return (
    <div className="flex h-full flex-col">
      <IconSprite />
      <TopBar />
      {/* relative: the composer overlay (narrow windows) and the draft pill are positioned here. */}
      <div className="relative flex min-h-0 flex-1">
        <WorkspaceRail />
        {settings ? <SettingsSidebar /> : <Sidebar />}
        <div className="flex min-w-0 flex-1 flex-col">
          {!isDemo && <WelcomeGate />}
          <main className="relative min-h-0 min-w-0 flex-1 overflow-hidden bg-paper">
            {empty && !setup && !settings ? <NoWorkspace /> : <Outlet />}
          </main>
          <TerminalDock />
        </div>
        {askOpen && !empty && <Suspense fallback={null}><AskPanel /></Suspense>}
        <Composer />
      </div>
      <StatusBar />
      <Toasts />
      {isDemo && <Suspense fallback={null}><TourRoot /></Suspense>}
      {addOpen && <Suspense fallback={null}><AddFolderDialog /></Suspense>}
    </div>
  );
}

export function EmptyView({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="grid h-full place-items-center p-8">
      <div className="max-w-md text-center">
        <h2 className="m-0 text-[15px] font-semibold">{title}</h2>
        <p className="mt-2 text-ink-2">{children}</p>
      </div>
    </div>
  );
}
