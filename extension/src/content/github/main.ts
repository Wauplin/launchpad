// The GitHub layer's loop: at each navigation, the page kind from the URL, the widgets rendered at once from the last
// context of the same repository (so the badge shows within GitHub's render), then Home's context for this page
// through the worker (cached there 30 s), then the widgets again. Bus changes arrive as the worker's `ext.context`
// topic. Widgets are idempotent: renders only change what changed.

import type { ExtContext } from "@/gen/ExtContext";
import { CONTEXT_TOPIC, request, subscribe, type Phase } from "../../shared/messages";
import { beginPage } from "./anchors";
import { observePage, watchNavigation } from "./navigation";
import { pageOf, type Page } from "./page";
import { renderRepoBadge } from "./repoBadge";
import { unmountAll } from "./widgets";

let page: Page = pageOf(location.href);
let context: ExtContext | null = null;
/** The last context of each repository (lowercase `owner/name`): what the next page of it shows until Home answers. */
const byRepo = new Map<string, ExtContext>();
let phase: Phase | null = null;
let generation = 0;

function render() {
  renderRepoBadge(page, context);
}

async function load(fresh = false) {
  const mine = ++generation;
  if (!page.repo) return;
  try {
    const data = await request("context", { url: page.url, fresh });
    if (mine !== generation) return;
    context = data;
    byRepo.set(page.repo.toLowerCase(), data);
    render();
  } catch {
    // Not connected, or Home did not answer: nothing on the page (the side panel says why).
    if (mine === generation && !context) unmountAll();
  }
}

function navigated() {
  page = pageOf(location.href);
  beginPage(page.kind, page.repo ? { repo: page.repo, number: page.number } : null);
  context = page.repo ? byRepo.get(page.repo.toLowerCase()) ?? null : null;
  if (!page.repo) { generation++; unmountAll(); return; }
  render();
  observePage(render);
  const mine = generation;
  // A fresh tab: the worker's context of another page of this repository, until Home answers for this one.
  if (!context) void request("context.peek", { url: page.url }).then((c) => { if (c && mine === generation && !context) { context = c; render(); } }).catch(() => undefined);
  void load();
}

/** The script starts while GitHub's HTML still loads (`document_start`): Home's context loads meanwhile, and the
 * widgets are tried every 50 ms until the page is parsed, then follow the scoped observer. */
function whileLoading() {
  if (document.readyState !== "loading") return;
  const timer = setInterval(render, 50);
  document.addEventListener("DOMContentLoaded", () => { clearInterval(timer); render(); observePage(render); }, { once: true });
}

export function start() {
  watchNavigation((sameUrl) => {
    if (sameUrl) { render(); observePage(render); } else navigated();
  });
  subscribe([CONTEXT_TOPIC], {
    state: (s) => {
      const was = phase;
      phase = s.phase;
      if (s.phase === "online" && was !== "online") void load();
      if (!s.home || s.phase === "revoked" || s.phase === "signed-out") { context = null; byRepo.clear(); unmountAll(); }
    },
    event: (e) => {
      const urls = (e.payload as { urls?: string[] | "*" } | null)?.urls;
      const here = new URL(page.url);
      here.hash = "";
      if (urls === "*" || (Array.isArray(urls) && urls.includes(here.toString()))) void load();
    },
    resync: () => void load(true),
  });
  navigated();
  whileLoading();
}
