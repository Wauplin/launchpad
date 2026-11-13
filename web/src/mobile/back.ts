// Android back on the phone. One rule: back and ‹ do the same thing: close the topmost overlay, else go one page
// back. Every overlay (menu, sheet, the composer, a full-screen mode) owns exactly one history entry: opening it
// pushes an entry with the same address and a marker (`useBackClose`); a back that leaves the marker closes it;
// closing it with ✕, a tap outside or Escape pops the entry. An action inside an overlay that navigates leaves its
// marker below the new page; a back that lands on such a stale marker goes back once more (same address, so the
// step is invisible). Detail views keep their state in the address (`?thread=`, `?note=`, `?file=`…), ‹ goes back
// (`usePhoneBack`), tab taps replace the entries above Today (`goTab`), and a cold start on another page puts
// Today under it (`installBack`). The laptop shell never installs this: every hook is a no-op there.

import { useEffect, useRef } from "react";
import type { RouterHistory } from "@tanstack/react-router";

const MARK = "__lpOverlay";
type Overlay = { id: string; index: number; close: () => void };

let hist: RouterHistory | null = null;
const open: Overlay[] = [];
/** Pops this module started itself (closing with ✕): the overlay is already closed. */
let ownPops = 0;
let lastIndex = 0;
let counter = 0;
/** A tab tap waiting for its `go` to land on the entry above Today, which it then replaces. */
let pendingTab: string | null = null;
/** `phoneBackTo` going back until it reaches its page. */
let pendingPage: { is: (path: string, search: URLSearchParams) => boolean; fallback: string } | null = null;
/** `replaceUnderOverlays` going back past the overlays' entries, then replacing the page under them. */
let pendingReplace: string | null = null;

const indexOf = (h: RouterHistory) => (h.location.state as { __TSR_index?: number }).__TSR_index ?? 0;
const markerOf = (h: RouterHistory) => (h.location.state as unknown as Record<string, unknown>)[MARK] as string | undefined;

/** Path and search of the page; the parent used when a page was opened cold, from a link. */
function parentOf(path: string, search: URLSearchParams): string | null {
  if (path === "/") return null;
  if (path.startsWith("/agents")) return search.has("session") ? "/agents" : null;
  // A bot's About page (`?panel=`) has the bot's chat under it.
  if (path.startsWith("/bots/")) return search.has("panel") ? path : "/chats";
  // A channel's Members page (`?panel=members`) has the channel under it.
  if (path.startsWith("/channels/")) return search.has("panel") ? path : "/chats";
  if (path === "/bots") return search.has("new") || search.has("archived") ? "/chats" : null;
  if (path.startsWith("/github/")) return "/inbox";
  if (path.startsWith("/inbox")) return search.has("thread") ? "/inbox" : null;
  if (path.startsWith("/topics/")) return "/topics";
  if (path === "/topics") return "/more";
  if (path.startsWith("/knowledge")) return search.has("note") ? "/knowledge" : "/more";
  if (path.startsWith("/projects/")) return "/projects";
  if (path.startsWith("/settings/")) return "/settings";
  if (["/projects", "/settings", "/schedules", "/setup", "/terminals"].includes(path)) return "/more";
  return null;
}

/** Called once by the phone shell. Returns the cleanup. */
export function installBack(h: RouterHistory): () => void {
  hist = h;
  // A reload on an overlay's entry: the overlay is gone, so the entry is a duplicate of the page under it.
  if (markerOf(h)) h.back();
  else if (indexOf(h) === 0) {
    // A cold start (an installed app opened from a Telegram link): Today, then the page's list, then the page,
    // so back walks up instead of closing the app.
    const url = new URL(h.location.href, location.origin);
    const here = url.pathname + url.search + url.hash;
    if (url.pathname !== "/") {
      const parent = parentOf(url.pathname, url.searchParams);
      // The browser history batches calls within a task: flush each one so every entry exists.
      h.replace("/"); h.flush();
      if (parent && parent !== "/" && parent !== url.pathname + url.search) { h.push(parent); h.flush(); }
      h.push(here); h.flush();
    }
  }
  lastIndex = indexOf(h);
  const off = h.subscribe(() => {
    const index = indexOf(h);
    const backwards = index < lastIndex;
    lastIndex = index;
    // Back past an overlay's entry closes it (innermost first).
    for (let i = open.length - 1; i >= 0; i--) {
      if (open[i].index > index) { const [o] = open.splice(i, 1); o.close(); }
    }
    if (pendingTab !== null && backwards) { const to = pendingTab; pendingTab = null; h.replace(to); return; }
    if (pendingReplace !== null && backwards) {
      if (markerOf(h) && index > 0) { h.back(); return; }
      const to = pendingReplace; pendingReplace = null; h.replace(to); return;
    }
    if (pendingPage && backwards) {
      const url = new URL(h.location.href, location.origin);
      if (markerOf(h) || !pendingPage.is(url.pathname, url.searchParams)) {
        if (index > 0) { h.back(); return; }
        h.replace(pendingPage.fallback);
      }
      pendingPage = null;
      return;
    }
    if (ownPops > 0 && backwards) ownPops--;
    // A stale marker (its overlay closed by an action that then navigated, or by its own ✕ over another stale
    // marker): skip it, so the next back never lands on the same page.
    const mark = markerOf(h);
    if (backwards && mark && !open.some((o) => o.id === mark)) h.back();
  });
  return () => { off(); hist = null; };
}

/** Whether the phone back mechanism runs (the phone shell is mounted). */
export const backInstalled = () => hist !== null;

/**
 * Gives an overlay its history entry while `isOpen`: Android back calls `close`. The overlay closes itself as usual
 * (✕, outside tap, an action); this hook pops the entry when it is still the current one, unless the close came
 * with a navigation (then the entry stays below the new page and is skipped by back).
 */
export function useBackClose(isOpen: boolean, close: () => void) {
  const closeRef = useRef(close);
  closeRef.current = close;
  useEffect(() => {
    const h = hist;
    if (!isOpen || !h) return;
    const id = `o${++counter}`;
    h.push(h.location.href, { ...(h.location.state as object), [MARK]: id });
    const entry: Overlay = { id, index: indexOf(h), close: () => closeRef.current() };
    open.push(entry);
    return () => {
      const at = open.indexOf(entry);
      if (at < 0) return; // Closed by back.
      open.splice(at, 1);
      // Let a navigation started by the same click commit first: then the entry is no longer the current one.
      setTimeout(() => {
        if (hist === h && markerOf(h) === id) { ownPops++; h.back(); }
      }, 0);
    };
  }, [isOpen]);
}

/** ‹ on a phone page: one step back when the previous entry is this window's, else replace the page with `parent`. */
export function phoneBack(parent: string) {
  const h = hist;
  if (!h) return;
  if (indexOf(h) > 0) h.back();
  else h.replace(parent);
}

/**
 * Leaves the overlays and pages above the nearest entry that `is` (for example the bot's chat under its About page,
 * with a menu and a sheet still open over it), going back one entry at a time; with none, replaces the page with
 * `fallback`. The open overlays close as their entries go.
 */
export function phoneBackTo(is: (path: string, search: URLSearchParams) => boolean, fallback: string) {
  const h = hist;
  if (!h) return;
  // Their entries go with this walk: closing them must not pop once more.
  for (const o of open.splice(0)) o.close();
  if (indexOf(h) === 0) { h.replace(fallback); return; }
  pendingPage = { is, fallback };
  h.back();
}

/**
 * A switch between siblings from inside an overlay (a bot's thread picked in its Threads sheet): the open overlays
 * close with their entries, then `to` replaces the page under them, so back never returns to the previous sibling.
 * Returns false when the shell is not installed (navigate with `replace`).
 */
export function replaceUnderOverlays(to: string): boolean {
  const h = hist;
  if (!h) return false;
  for (const o of open.splice(0)) o.close();
  if (!markerOf(h) || indexOf(h) === 0) { h.replace(to); return true; }
  pendingReplace = to;
  h.back();
  return true;
}

/**
 * A bottom tab tap: Today is the base entry; another tab replaces whatever is above Today, so back from any tab
 * goes to Today and back on Today leaves the app. Returns false when the shell is not installed (use a link).
 */
export function goTab(to: string): boolean {
  const h = hist;
  if (!h) return false;
  const index = indexOf(h);
  if (to === "/") {
    if (index > 0) h.go(-index);
    else if (h.location.pathname !== "/" || h.location.search) h.replace("/");
    return true;
  }
  if (index <= 0) { if (h.location.pathname === "/" && !markerOf(h)) h.push(to); else h.replace(to); return true; }
  if (index === 1) { h.replace(to); return true; }
  // Back to the entry above Today, then replace it once the browser got there (`installBack`'s listener).
  pendingTab = to;
  h.go(1 - index);
  return true;
}
