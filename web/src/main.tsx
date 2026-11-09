import { delegatedAccess, loadDeployment } from "@/lib/deployment";
import { SaveStatus } from "@/components/SaveStatus";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { HarnessProvider } from "@/components/HarnessMark";
import { makeRouter, type Entry } from "@/app/router";
import { isPhone, watchLayout } from "@/lib/phone";
import { persistCache, restoreCache } from "@/lib/offlineCache";
import { logToDaemon } from "@/lib/api";
import { connectEvents } from "@/lib/ws";
import { useComposer } from "@/stores/composer";
import { isDemo } from "@/demo/mode";
// Reads `?ws=` before anything else rewrites the address.
import "@/stores/workspace";
import "./styles.css";
import darkMarkdown from "github-markdown-css/github-markdown-dark.css?inline";

// Scope GitHub's full dark stylesheet to our explicit choice, independent of OS theme.
// Put it before app styles so our Markdown size and transparent surface still win.
const markdownTheme = document.createElement("style");
markdownTheme.textContent = darkMarkdown.replaceAll(".markdown-body", ':root[data-theme="dark"] .markdown-body');
document.head.prepend(markdownTheme);

const queryClient = new QueryClient({
  defaultOptions: {
    // Data is pushed over the WebSocket or refreshed explicitly; never refetch just because the window got focus.
    queries: { refetchOnWindowFocus: false, retry: 1, staleTime: 30_000 },
  },
});
// Demo mode answers every request in the browser (src/demo/); its chunk loads only then.
const demo = isDemo ? import("@/demo/boot").then((m) => m.bootDemo(queryClient)) : Promise.resolve();
void demo.then(() => useComposer.getState().restore());
let navigateTo: ((url: string) => void) | null = null;
const deployment = demo.then(() => loadDeployment());
// Earlier versions registered a service worker (offline shell and Web Push); remove it and its caches.
if (!isDemo && "serviceWorker" in navigator) {
  void navigator.serviceWorker.getRegistrations().then((all) => all.forEach((r) => void r.unregister())).catch(() => {});
  if ("caches" in window) void caches.keys().then((names) => names.filter((n) => n.startsWith("launchpad-")).forEach((n) => void caches.delete(n))).catch(() => {});
}

window.addEventListener("error", (e) => logToDaemon(`uncaught: ${e.message} at ${e.filename}:${e.lineno}`, "error"));
window.addEventListener("unhandledrejection", (e) => logToDaemon(`unhandled rejection: ${String(e.reason)}`, "error"));

// Each shell is its own chunk: a phone never downloads the laptop shell, and the reverse.
// index.html guessed the layout before paint; the window may have been resized since (a window restoring
// its size), so `isPhone` decides: a stale `lp-phone` would apply the phone CSS to the laptop shell.
document.documentElement.classList.toggle("lp-phone", isPhone);
watchLayout();
const entry: Promise<Entry> = isPhone ? import("@/mobile/entry").then((m) => m.entry) : import("@/app/desktop").then((m) => m.entry);
// Browser tabs draw from the last saved lists while the daemon answers (lib/offlineCache.ts).
const cache = isDemo ? demo : restoreCache(queryClient).then(() => persistCache(queryClient));
// Restore cached lists before the first socket resync invalidates them.
void cache.then(() => connectEvents(queryClient));
void Promise.all([entry, cache, deployment]).then(([pages]) => {
  const router = makeRouter(pages);
  navigateTo = (url) => router.history.push(url);
  // Demo recordings (videos, screenshots) drive the router without a page load.
  if (isDemo) window.__lpDemo!.navigate = navigateTo;
  if (delegatedAccess() && window.parent !== window) {
    const notifyHost = () => window.parent.postMessage({ type: "launchpad-route", route: router.history.location.href }, location.origin);
    router.history.subscribe(notifyHost);
    notifyHost();
  }
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <QueryClientProvider client={queryClient}>
        <SaveStatus /><HarnessProvider><RouterProvider router={router} /></HarnessProvider>
      </QueryClientProvider>
    </StrictMode>,
  );
});
