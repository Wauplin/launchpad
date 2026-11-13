// The GitHub layer's loop: at each navigation, the page kind from the URL, the widgets rendered at once from the last
// context of the same repository (so the badge shows within GitHub's render), then Home's context for this page
// through the worker (cached there 30 s), then the widgets again. Bus changes arrive as the worker's `ext.context`
// topic. Widgets are idempotent: renders only change what changed.

import type { ExtContext } from "@/gen/ExtContext";
import { CONTEXT_TOPIC, request, subscribe, type Phase, type TabCommand } from "../../shared/messages";
import { beginPage } from "./anchors";
import { renderComments, watch } from "./comments";
import { insertFromPanel, renderInsertBanners } from "./insert";
import { isItem } from "./item";
import { clearLists, listsStale, renderLists } from "./lists";
import { clearNotifications, notificationsStale, renderNotifications, watchNotificationForms } from "./notifications";
import { observePage, watchNavigation } from "./navigation";
import { pageOf, type Page } from "./page";
import { renderRepoBadge } from "./repoBadge";
import { renderRepoStrip } from "./repoStrip";
import { renderSidebarBox } from "./sidebarBox";
import { unmount, unmountAll } from "./widgets";

let page: Page = pageOf(location.href);
let context: ExtContext | null = null;
/** The last context of each repository (lowercase `owner/name`): what the next page of it shows until Home answers. */
const byRepo = new Map<string, ExtContext>();
let phase: Phase | null = null;
let generation = 0;
/** Home's context for this page is on its way: the sidebar box keeps its space meanwhile. */
let loading = false;

/** Connected to Home (the list pages ask for rows only then). */
const connected = () => phase === "online" || phase === "offline" || phase === "starting";

function render() {
  if (page.kind === "notifications") {
    if (connected()) renderNotifications(page, render); else clearNotifications();
    return;
  }
  renderRepoBadge(page, context, refresh);
  renderRepoStrip(page, context, refresh);
  if (connected()) renderLists(page, context, render, refresh); else clearLists();
  if (isItem(page)) {
    const ctx = context && context.page.number === page.number && context.page.repo?.toLowerCase() === page.repo.toLowerCase() ? context : null;
    if (ctx || (loading && phase !== "signed-out" && phase !== "revoked")) renderSidebarBox(page, ctx, refresh);
    else unmount("sidebar-box");
    watch(renderComments(page, ctx, refresh), render);
    renderInsertBanners(page, ctx, refresh);
  } else {
    unmount("sidebar-box");
    watch([], render);
  }
}

/** After an action of the page (a bot asked, an agent stopped): show it at once, then Home's fresh context. */
function refresh() {
  render();
  void load(true);
}

async function load(fresh = false) {
  const mine = ++generation;
  if (!page.repo) return;
  loading = true;
  try {
    const data = await request("context", { url: page.url, fresh });
    if (mine !== generation) return;
    context = data;
    byRepo.set(page.repo.toLowerCase(), data);
    render();
  } catch {
    // Not connected, or Home did not answer: nothing on the page (the side panel says why).
    if (mine === generation && !context) unmountAll();
  } finally {
    if (mine === generation) loading = false;
  }
}

function navigated() {
  page = pageOf(location.href);
  beginPage(page.kind, page.repo ? { repo: page.repo, number: page.number } : page.kind === "notifications" ? { repo: "", number: null } : null);
  context = page.repo ? byRepo.get(page.repo.toLowerCase()) ?? null : null;
  clearLists();
  if (page.kind === "notifications") { generation++; unmountAll(); clearNotifications(); render(); observePage(render); return; }
  clearNotifications();
  if (!page.repo) { generation++; unmountAll(); return; }
  render();
  observePage(render);
  void load();
  // `load` took its generation synchronously: the peek counts only until another navigation or load starts.
  const mine = generation;
  // A fresh tab: the worker's context of another page of this repository, until Home answers for this one.
  if (!context) void request("context.peek", { url: page.url }).then((c) => { if (c && mine === generation && !context) { context = c; render(); } }).catch(() => undefined);
}

/** The script starts while GitHub's HTML still loads (`document_start`): Home's context loads meanwhile, and the
 * widgets are tried every 50 ms until the page is parsed, then follow the scoped observer. */
function whileLoading() {
  if (document.readyState !== "loading") return;
  const timer = setInterval(render, 50);
  document.addEventListener("DOMContentLoaded", () => { clearInterval(timer); render(); observePage(render); }, { once: true });
}

/** The panel's buttons for this tab (TabCommand): the same actions as the page's. */
function commands() {
  chrome.runtime.onMessage.addListener((m: TabCommand, sender, reply: (r: { ok: boolean; error?: string }) => void) => {
    if (sender.id !== chrome.runtime.id || sender.tab || m?.type !== "insert") return false;
    insertFromPanel(isItem(page) ? page : null, context, String(m.suggestion))
      .then(() => reply({ ok: true }), (e) => reply({ ok: false, error: e instanceof Error ? e.message : String(e) }));
    return true;
  });
}

export function start() {
  commands();
  watchNotificationForms(() => page.kind === "notifications");
  // GitHub's own list checkboxes change no markup on every layout: L4's selection bar follows them here.
  document.addEventListener("change", (e) => { if ((page.kind === "pulls" || page.kind === "issues") && e.target instanceof HTMLInputElement && e.target.type === "checkbox") render(); }, true);
  // Live cards count their minutes.
  setInterval(() => { if (isItem(page) && document.visibilityState === "visible") render(); }, 30_000);
  watchNavigation((sameUrl) => {
    if (sameUrl) { render(); observePage(render); } else navigated();
  });
  subscribe([CONTEXT_TOPIC], {
    state: (s) => {
      const was = phase;
      phase = s.phase;
      if (s.phase === "online" && was !== "online") { notificationsStale(); listsStale(); if (page.kind === "notifications") render(); else void load(); }
      if (!s.home || s.phase === "revoked" || s.phase === "signed-out") { context = null; byRepo.clear(); clearLists(); clearNotifications(); unmountAll(); }
    },
    event: (e) => {
      const urls = (e.payload as { urls?: string[] | "*" } | null)?.urls;
      // Rows and the notifications strip are not cached by the worker: any change of Home's records asks again.
      notificationsStale();
      listsStale();
      if (page.kind === "notifications" || page.kind === "pulls" || page.kind === "issues") render();
      const here = new URL(page.url);
      here.hash = "";
      if (urls === "*" || (Array.isArray(urls) && urls.includes(here.toString()))) void load();
    },
    resync: () => { notificationsStale(); listsStale(); if (page.kind === "notifications") render(); else void load(true); },
  });
  navigated();
  whileLoading();
}
