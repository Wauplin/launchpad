import { delegatedAccess, loadDeployment } from "@/lib/deployment";
import { SaveStatus } from "@/components/SaveStatus";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { HarnessProvider } from "@/components/HarnessMark";
import { makeRouter, type Entry } from "@/app/router";
import { isPhone, watchLayout } from "@/lib/phone";
import { isDesktopShell } from "@/lib/platform";
import { persistCache, restoreCache } from "@/lib/offlineCache";
import { logToDaemon } from "@/lib/api";
import { registerServiceWorker } from "@/lib/push";
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
// Notification clicks open their route in this window through the router.
let navigateTo: ((url: string) => void) | null = null;
const deployment = demo.then(() => loadDeployment());
if (!isDemo) void deployment.then(() => registerServiceWorker((url) => (navigateTo ? navigateTo(url) : location.assign(url))));

window.addEventListener("error", (e) => logToDaemon(`uncaught: ${e.message} at ${e.filename}:${e.lineno}`, "error"));
window.addEventListener("unhandledrejection", (e) => logToDaemon(`unhandled rejection: ${String(e.reason)}`, "error"));

// Each shell is its own chunk: a phone never downloads the desktop shell, and the reverse.
watchLayout();
const entry: Promise<Entry> = isPhone ? import("@/mobile/entry").then((m) => m.entry) : import("@/app/desktop").then((m) => m.entry);
// Browser tabs draw from the last saved lists while the daemon answers (lib/offlineCache.ts).
const cache = isDemo ? demo : isDesktopShell ? Promise.resolve() : restoreCache(queryClient).then(() => persistCache(queryClient));
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

// The desktop shell passes its start time (`lp_t0`, epoch ms) to measure the time to the first frame.
const params = new URLSearchParams(location.search);
const t0 = Number(params.get("lp_t0"));
if (t0) {
  params.delete("lp_t0");
  const rest = params.toString();
  history.replaceState(null, "", location.pathname + (rest ? `?${rest}` : "") + location.hash);
  requestAnimationFrame(() =>
    requestAnimationFrame(() =>
      logToDaemon(`window ready: ${Date.now() - t0} ms after the shell started (page ${Math.round(performance.now())} ms)`),
    ),
  );
}
