// The folio reader alone, for a native client's WebView (the Android app): `/folio-embed/index.html?ws=&slug=`
// (`&page=&anchor=&rev=&theme=`). It is Home's own reader (features/folios) from the same build as the running Home,
// without the app shell and without the reader's top bar: the host draws the title, the pages and Send, and this
// page keeps the folio, its marks, the comment and Send sheets.
//
// - Requests: GET stays a plain fetch, which the host intercepts to add its credentials; every write goes through
//   `window.LaunchpadNative.request` (lib/api.ts). The page never holds a secret, opens no WebSocket and registers no
//   service worker.
// - To the host, `window.LaunchpadNative.event(json)`: `state` (title, kind, status, rev, latest_rev, page, pages,
//   unsent, anchor), `open` (a link the host opens: an external address, or a Launchpad page on Home's origin),
//   `overlay` (how many sheets are open: the host's Back sends `back` while it is above 0), `ready`, `error`.
// - From the host, `window.__launchpadNative.command(json)`: `page`, `anchor`, `send`, `comments`, `refresh`, `theme`,
//   `back`.
// - A link never leaves the page: the router here has one route and a memory history, where each sheet owns an entry
//   (mobile/back.ts) and any other navigation becomes an `open` event.

import { StrictMode, useCallback, useEffect } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider, createMemoryHistory, createRootRoute, createRouter } from "@tanstack/react-router";
import { create } from "zustand";
import darkMarkdown from "github-markdown-css/github-markdown-dark.css?inline";
import { IconSprite } from "@/components/Icon";
import { ErrorState } from "@/components/ui";
import { Toasts } from "@/components/Toast";
import { FolioReader, type FolioHost, type Panel } from "@/features/folios/FolioReader";
import { installNative } from "@/lib/api";
import { installBack } from "@/mobile/back";
import { useWorkspace } from "@/stores/workspace";
import "./embed.css";

type Params = { ws: string; slug: string; page: string | null; anchor: string | null; rev: string | null; theme: string | null };
const params = (window as unknown as { __lpEmbed: Params }).__lpEmbed;

const markdownTheme = document.createElement("style");
markdownTheme.textContent = darkMarkdown.replaceAll(".markdown-body", ':root[data-theme="dark"] .markdown-body');
document.head.prepend(markdownTheme);

const emit = (event: Record<string, unknown>) => { try { window.LaunchpadNative?.event(JSON.stringify(event)); } catch { /* The host went away. */ } };

type Nav = { page?: string; rev?: number; panel?: Panel; section?: string };
/** What the reader shows. Kept outside React, so it survives anything the router does. */
const useNav = create<{ nav: Nav }>(() => ({ nav: { page: params.page ?? undefined, rev: params.rev ? Number(params.rev) || undefined : undefined, section: params.anchor ?? undefined } }));
const setNav = (n: Nav) => useNav.setState((s) => ({ nav: { ...s.nav, ...n } }));

const queryClient = new QueryClient({ defaultOptions: { queries: { retry: 1, refetchOnWindowFocus: false } } });
// The reader's scope (folders for a launch prompt) is this workspace.
useWorkspace.setState({ current: params.ws, requested: null });

let shown: Record<string, unknown> = {};
let anchor: string | null = params.anchor;
const tell = () => emit({ type: "state", ...shown, anchor });
let commands: { send: () => void; comments: () => void; top: () => void } | null = null;

const host: FolioHost = {
  state: (s) => { shown = s; tell(); },
  anchor: (id) => { anchor = id; if (shown.page) tell(); },
  commands: (c) => { commands = c; },
  ready: () => emit({ type: "ready" }),
  error: (message) => emit({ type: "error", message }),
};

const jump = (id: string) => document.getElementById(`f-${id}`)?.scrollIntoView({ block: "start" });

// One route, a memory history: sheets push their own entries; anything else is a link for the host.
const history = createMemoryHistory({ initialEntries: ["/"] });
const router = createRouter({ routeTree: createRootRoute({ component: Embed }), history });
installBack(history);
const depth = () => (history.location.state as { __TSR_index?: number }).__TSR_index ?? 0;
let overlays = 0;
history.subscribe(() => {
  const at = history.location;
  if (at.pathname !== "/") {
    emit({ type: "open", url: location.origin + at.href });
    history.back();
    return;
  }
  if (depth() !== overlays) { overlays = depth(); emit({ type: "overlay", open: overlays }); }
});

installNative((json) => {
  let c: { type?: string; page?: string; id?: string; theme?: string };
  try { c = JSON.parse(json); } catch { return; }
  if (c.type === "page" && c.page) { setNav({ page: c.page, section: undefined }); commands?.top(); }
  else if (c.type === "anchor" && c.id) jump(c.id);
  else if (c.type === "send") commands?.send();
  else if (c.type === "comments") commands?.comments();
  else if (c.type === "refresh") void queryClient.invalidateQueries({ queryKey: ["folio", params.ws, params.slug] });
  else if (c.type === "theme" && (c.theme === "dark" || c.theme === "light")) document.documentElement.dataset.theme = c.theme;
  else if (c.type === "back" && depth() > 0) history.back();
});

// A link with an address (external pages, files) never navigates this page: the host opens it.
document.addEventListener("click", (e) => {
  const a = (e.target as Element | null)?.closest?.("a[href]") as HTMLAnchorElement | null;
  if (!a || a.hasAttribute("data-folio-link")) return;
  const href = a.getAttribute("href") ?? "";
  if (!href || href.startsWith("#")) return;
  e.preventDefault();
  if (/^(https?:|mailto:)/.test(href)) { e.stopPropagation(); emit({ type: "open", url: href }); }
  // Other links are the router's (a session, a bot's thread, a topic): its history turns them into `open`.
}, true);
window.open = (url) => { if (url) emit({ type: "open", url: new URL(String(url), location.href).href }); return null; };

function Embed() {
  const nav = useNav((s) => s.nav);
  const onNav = useCallback((n: Nav) => setNav(n), []);
  useEffect(() => { if (!params.ws || !params.slug) emit({ type: "error", message: "The address names no folio (?ws=&slug=)" }); }, []);
  if (!params.ws || !params.slug) return <ErrorState error={new Error("The address names no folio (?ws=&slug=)")} />;
  return <div className="h-dvh bg-paper"><IconSprite /><FolioReader ws={params.ws} slug={params.slug} nav={nav} onNav={onNav} host={host} /><Toasts /></div>;
}

createRoot(document.getElementById("root")!).render(<StrictMode><QueryClientProvider client={queryClient}><RouterProvider router={router as never} /></QueryClientProvider></StrictMode>);
