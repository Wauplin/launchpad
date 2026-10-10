import { Outlet } from "@tanstack/react-router";
import { IconSprite } from "@/components/Icon";
import { Toasts } from "@/components/Toast";
import { Composer } from "@/features/launch/Composer";
import { TerminalDock } from "@/features/terminals/TerminalDock";
import { useConfig } from "@/lib/queries";
import { useLayoutEffect } from "react";
import { zoomScale } from "@/lib/zoom";
import { useWorkspaceSync } from "@/lib/workspace";
import { Sidebar } from "./Sidebar";
import { StatusBar } from "./StatusBar";
import { TopBar } from "./TopBar";

export function Shell() {
  const config = useConfig().data?.config;
  useLayoutEffect(() => { document.documentElement.style.zoom = String(zoomScale(config?.zoom ?? 2)); }, [config?.zoom]);
  useWorkspaceSync();
  return (
    <div className="flex h-full flex-col">
      <IconSprite />
      <TopBar />
      <div className="flex min-h-0 flex-1">
        <Sidebar />
        <div className="flex min-w-0 flex-1 flex-col">
          <main className="relative min-h-0 min-w-0 flex-1 overflow-hidden bg-paper">
            <Outlet />
          </main>
          <TerminalDock />
        </div>
        <Composer />
      </div>
      <StatusBar />
      <Toasts />
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
