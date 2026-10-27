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
import { NotificationsPrompt } from "./NotificationsPrompt";

const AddFolderDialog = lazy(() => import("@/features/projects/AddFolder").then((m) => ({ default: m.AddFolderDialog })));

export function Shell() {
  const addOpen = useAddFolder((s) => s.open);
  const scale = useZoomScale();
  useLayoutEffect(() => { document.documentElement.style.zoom = String(scale); }, [scale]);
  useWorkspaceSync();
  return (
    <div className="flex h-full flex-col">
      <IconSprite />
      <TopBar />
      {/* relative: the composer overlay (narrow windows) and the draft pill are positioned here. */}
      <div className="relative flex min-h-0 flex-1">
        <Sidebar />
        <div className="flex min-w-0 flex-1 flex-col">
          <NotificationsPrompt />
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
