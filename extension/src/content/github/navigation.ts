// Soft navigation: GitHub changes pages without loading them (Turbo and React routes). The layer listens to the
// Navigation API, Turbo's events and popstate, with URL polling as the fallback, and re-renders its widgets (which are
// idempotent, keyed by anchor and item). Between navigations, one MutationObserver scoped to the page body (never the
// whole document) notices when GitHub re-renders the parts the widgets sit next to.

import { ROOTS } from "./anchors";

/** Calls `changed(sameUrl)` after each navigation; `sameUrl` is true when Turbo rendered the same address again. */
export function watchNavigation(changed: (sameUrl: boolean) => void) {
  let last = location.href;
  const check = (rendered = false) => {
    const moved = location.href !== last;
    if (!moved && !rendered) return;
    last = location.href;
    changed(!moved);
  };
  const nav = (window as unknown as { navigation?: EventTarget }).navigation;
  nav?.addEventListener("navigatesuccess", () => check());
  document.addEventListener("turbo:load", () => check(true));
  document.addEventListener("turbo:render", () => check(true));
  window.addEventListener("popstate", () => check());
  setInterval(() => check(), 1_000);
}

let observer: MutationObserver | null = null;

/** Watches the current page's body for changes (one observer at a time, replaced at each navigation) and calls
 * `changed` at most once per frame. */
export function observePage(changed: () => void) {
  observer?.disconnect();
  const root = ROOTS.map((s) => document.querySelector(s)).find((el): el is Element => !!el && el !== document.documentElement);
  if (!root) return;
  let queued = false;
  observer = new MutationObserver((records) => {
    // Our own widgets changing is not GitHub re-rendering.
    if (queued || records.every((r) => (r.target as Element).closest?.("[data-launchpad]"))) return;
    queued = true;
    requestAnimationFrame(() => { queued = false; changed(); });
  });
  observer.observe(root, { childList: true, subtree: true });
}
