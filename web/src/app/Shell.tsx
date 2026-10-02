import { Outlet } from "@tanstack/react-router";
import { IconSprite } from "@/components/Icon";
import { Sidebar } from "./Sidebar";
import { StatusBar } from "./StatusBar";
import { TopBar } from "./TopBar";

export function Shell() {
  return (
    <div className="flex h-full flex-col">
      <IconSprite />
      <TopBar />
      <div className="flex min-h-0 flex-1">
        <Sidebar />
        <main className="relative min-w-0 flex-1 overflow-hidden bg-paper">
          <Outlet />
        </main>
      </div>
      <StatusBar />
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
