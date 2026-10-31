// Starts demo mode (lazy chunk, loaded by main.tsx only when `isDemo`): builds the world, registers the route
// handlers, replaces WebSocket for daemon sockets and releases the requests that index.html held back.

import type { QueryClient } from "@tanstack/react-query";
import { isPhone } from "@/lib/phone";
import { clock, demoFetch, FakeSocket } from "./daemon";
import { demoAsset } from "./assets";
import { demoChapter, exitDemo } from "./mode";
import { resetWorld, world } from "./world";
import { continueQuota, skipToEnd } from "./player";
import "./routes";

let queryClient: QueryClient | null = null;
/** The app's query client, so a chapter can reset the world and reload every query. */
export const demoQueryClient = () => queryClient!;

export async function bootDemo(qc: QueryClient) {
  // The tour and the demo exist only in the desktop layout; `?demo=phone` explores the phone shell (for screenshots and videos).
  if (isPhone && demoChapter() !== "phone") { exitDemo("/now"); return new Promise<never>(() => {}); }
  queryClient = qc;
  resetWorld();
  const Real = window.WebSocket;
  // Daemon sockets go to the demo; anything else (the Vite dev server) keeps the real WebSocket.
  const Socket = function (url: string | URL, protocols?: string | string[]) {
    const path = new URL(String(url).replace(/^ws/, "http")).pathname;
    if (path === "/ws" || path.startsWith("/api/")) return new FakeSocket(url);
    return new Real(url, protocols);
  } as unknown as typeof WebSocket;
  Object.assign(Socket, { CONNECTING: 0, OPEN: 1, CLOSING: 2, CLOSED: 3 });
  window.WebSocket = Socket;
  // No permission prompt, no Web Push: notifications stay in the page.
  if ("Notification" in window) Object.defineProperty(window, "Notification", { value: Object.assign(function () {}, { permission: "denied", requestPermission: async () => "denied" }), configurable: true });
  window.__lpDemo!.asset = demoAsset;
  // Scripted recordings (videos, screenshots) move the demo clock and finish runs like the tour does.
  window.__lpDemo!.control = { world: () => world, clock, continueQuota, skipToEnd };
  document.title = "Launchpad (demo)";
  new MutationObserver(() => { if (document.title !== "Launchpad (demo)") document.title = "Launchpad (demo)"; }).observe(document.querySelector("title")!, { childList: true });
  const url = new URL(location.href);
  if (url.searchParams.has("demo")) { url.searchParams.delete("demo"); history.replaceState(history.state, "", url.pathname + url.search + url.hash); }
  document.documentElement.classList.add("lp-demo");
  window.__lpDemo!.ready({ fetch: demoFetch });
}
