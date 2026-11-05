import { Outlet } from "@tanstack/react-router";
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
import { StatusBar } from "./StatusBar";
import { TopBar } from "./TopBar";
import { HostPrompts } from "./HostPrompt";
import { isDemo } from "@/demo/mode";
import { WelcomeGate } from "@/onboarding/WelcomeGate";

// Demo mode only: the tour overlay and its chapters (src/onboarding/tour/) load with the demo.
const TourRoot = lazy(() => import("@/onboarding/tour/TourRoot").then((m) => ({ default: m.TourRoot })));
const AddFolderDialog = lazy(() => import("@/features/projects/AddFolder").then((m) => ({ default: m.AddFolderDialog })));

/** Home is served directly in the Hugging Face Space page, whose mini header (Space name, menu) is a pill fixed 20 px
 * from the top and 24 px from the right, 40 px tall, hidden below 640 px. Its width follows the Space ID, whose
 * length matches the first label of the `*.hf.space` host. Keep the top bar's buttons out of it (see TopBar). A
 * parent Space shell (deploy/hf-space) sets these itself, so only a page embedded directly by the Hub does it. */
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
  const scale = useZoomScale();
  useLayoutEffect(() => {
    document.documentElement.style.zoom = String(scale);
    // Embedding hosts express insets in unzoomed pixels (see TopBar).
    document.documentElement.style.setProperty("--lp-zoom", String(scale));
  }, [scale]);
  useWorkspaceSync();
  useHubPillInset();
  return (
    <div className="flex h-full flex-col">
      <IconSprite />
      <TopBar />
      {/* relative: the composer overlay (narrow windows) and the draft pill are positioned here. */}
      <div className="relative flex min-h-0 flex-1">
        <Sidebar />
        <div className="flex min-w-0 flex-1 flex-col">
          {!isDemo && <WelcomeGate />}
          <main className="relative min-h-0 min-w-0 flex-1 overflow-hidden bg-paper">
            <Outlet />
          </main>
          <TerminalDock />
        </div>
        <Composer />
      </div>
      <StatusBar />
      <Toasts />
      <HostPrompts />
      {isDemo && <Suspense fallback={null}><TourRoot /></Suspense>}
      {addOpen && <Suspense fallback={null}><AddFolderDialog /></Suspense>}
    </div>
  );
}

export function EmptyView({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="grid h-full place-items-center p-8">
      <div className="max-w-md text-center">
        <h2 className="m-0 text-lg font-semibold">{title}</h2>
        <p className="mt-2 text-ink-2">{children}</p>
      </div>
    </div>
  );
}
