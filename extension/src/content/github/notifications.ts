// github.com/notifications (M2): N1 the Today strip of the extension's workspace above the list (to answer, reviews
// ready, suggestions to post, Local PRs; Open Today), N4 Done all handled (the threads a bot already merged, closed,
// approved or answered, listed in a confirmation; the confirm click marks them Done here and on GitHub), N2 one chip per
// row, N5 Review with agent / Ask on a row's hover, and N3: Done, mark as read and Unsubscribe clicked in GitHub's own
// list are recorded in Launchpad's Inbox at once (`inbox.observed`), observed only, never triggered.

import type { ExtNotifications } from "@/gen/ExtNotifications";
import { request, type ObservedAction } from "../../shared/messages";
import { addPopover, askWithoutCloning } from "./addRepo";
import { findAnchor, findAnchors, rowItem, notificationSlot } from "./anchors";
import { open } from "./item";
import type { Page } from "./page";
import { chipButton, chipOf, hoverWith, RowStates } from "./rows";
import { targetPopover, type Target } from "./target";
import { button, h, logo, more, mount, popover, unmount } from "./widgets";

const STRIP = "notifications-strip";
let strip: { data: ExtNotifications | null; loading: boolean; stale: boolean } = { data: null, loading: false, stale: true };
let states: RowStates | null = null;
let shown = new Set<string>();

export function notificationsStale() {
  strip.stale = true;
  states?.stale();
}

export function clearNotifications() {
  unmount(STRIP);
  for (const id of shown) unmount(id);
  shown = new Set();
  strip = { data: null, loading: false, stale: true };
  states?.clear();
}

export function renderNotifications(page: Page, rerender: () => void) {
  if (page.kind !== "notifications") return clearNotifications();
  if (strip.stale && !strip.loading) {
    strip.loading = true;
    void request("notifications").then((d) => { strip = { data: d, loading: false, stale: false }; rerender(); }).catch(() => { strip.loading = false; strip.stale = false; });
  }
  renderStrip(strip.data, rerender);
  states ??= new RowStates(rerender);
  renderRows(page, strip.data, states);
}

function renderStrip(d: ExtNotifications | null, rerender: () => void) {
  if (!d?.workspace) return unmount(STRIP);
  const anchor = findAnchor("notification-list", { repo: "", number: null });
  if (!anchor) return unmount(STRIP);
  const w = mount(STRIP, JSON.stringify(d), anchor.el, anchor.place, "banner");
  if (!w.fresh) return;
  const ws = d.workspace.id;
  const c = d.counts;
  const part = (n: number, text: string, filter: string, title: string) => n > 0 ? h("button", { class: "lp-link", text: `${n} ${text}`, title, attrs: { type: "button" }, on: { click: (e) => { e.preventDefault(); open(`/?filter=${filter}`, ws); } } }) : null;
  const parts = [
    part(c.answer, "to answer", "answer", "Agents' questions waiting for you"),
    part(c.review, c.review === 1 ? "agent to review" : "agents to review", "review", "Finished agents you have not seen"),
    part(c.suggestions, c.suggestions === 1 ? "suggestion to post" : "suggestions to post", "github", "Agent-written comments and reviews"),
    part(c.local_prs, c.local_prs === 1 ? "Local PR" : "Local PRs", "review", "Local PRs to publish"),
  ].filter((x): x is HTMLButtonElement => !!x);
  const today = button("Open Today", () => open("/", ws), { small: true, title: `Launchpad's Today of ${d.workspace.name}` });
  const top = h("div", { class: "lp-row", attrs: { style: "flex-wrap:wrap;gap:6px 14px" } }, logo(), h("span", { class: "lp-title", text: d.workspace.name }),
    ...(parts.length ? parts : [h("span", { class: "lp-muted", text: "Nothing waits for you in Launchpad" })]), h("span", { class: "lp-grow" }), today);
  const box = h("div", { class: "lp-banner" }, top);
  if (d.handled.length) {
    const doneAll: HTMLButtonElement = button(`Done all ${d.handled.length} handled…`, () => doneAllPopover(w.root, doneAll, d, rerender), { small: true, title: "Threads a bot already merged, closed, approved or answered" });
    box.append(h("div", { class: "lp-row" }, h("span", { class: "lp-muted lp-grow", text: `${d.handled.length} thread${d.handled.length === 1 ? "" : "s"} handled by bots` }), doneAll));
  }
  if (d.observed) box.append(h("div", { class: "lp-muted", text: `You marked ${d.observed} thread${d.observed === 1 ? "" : "s"} on GitHub today: Launchpad's Inbox is up to date.` }));
  w.root.append(box);
}

/** N4: the confirmation lists the threads; only its confirm click marks them Done (here and on GitHub). */
function doneAllPopover(root: ShadowRoot, anchor: HTMLElement, d: ExtNotifications, rerender: () => void) {
  const error = h("div", { class: "lp-error", attrs: { role: "alert", hidden: "" } });
  const go = h("button", { class: "lp-btn lp-primary", text: `Mark ${d.handled.length} Done`, attrs: { type: "button" } });
  const list = h("div", { attrs: { style: "display:grid;gap:4px;max-height:200px;overflow:auto" } }, ...d.handled.map((t) => h("div", { class: "lp-row", title: t.title }, h("span", { class: "lp-dot lp-ok" }), h("span", { class: "lp-grow", text: `${t.repo}#${t.number} ${t.title}` }), h("span", { class: "lp-muted", text: `${t.bot ?? "A bot"} · ${t.outcome}` }))));
  const close = popover(root, anchor, "Done all handled by bots", (close) => [
    list,
    h("div", { class: "lp-confirm", text: "Marks these threads Done in Launchpad and on GitHub." }),
    error,
    h("div", { class: "lp-row" }, h("span", { class: "lp-grow" }), button("Cancel", close, { small: true }), go),
  ]);
  go.addEventListener("click", (e) => {
    e.preventDefault();
    if (!e.isTrusted || go.disabled) return;
    go.disabled = true;
    go.textContent = "Marking Done…";
    request("inbox.done-handled", { ids: d.handled.map((t) => t.thread) }).then((results) => {
      const failed = results.filter((r) => r.error);
      notificationsStale();
      rerender();
      if (!failed.length) return close();
      error.textContent = `${failed.length} not marked: ${failed.map((r) => r.error).join("; ")}`;
      error.hidden = false;
      go.textContent = "Done";
    }).catch((err) => { error.textContent = err instanceof Error ? err.message : String(err); error.hidden = false; go.disabled = false; go.textContent = `Mark ${d.handled.length} Done`; });
  });
}

function renderRows(page: Page, d: ExtNotifications | null, st: RowStates) {
  const found = findAnchors("notification-rows", { repo: "", number: null });
  const rows = (found?.els ?? []).map((el) => ({ el, item: rowItem(el) })).filter((r) => r.item);
  const data = rows.length ? st.get(page.url, rows.map((r) => `${r.item!.repo}#${r.item!.number}`)) : null;
  const ws = d?.workspace?.id ?? null;
  const next = new Set<string>();
  for (const { el, item } of rows) {
    const key = `${item!.repo}#${item!.number}`;
    const id = `nrow-${key.replace(/[^A-Za-z0-9]/g, "-")}`;
    const slot = notificationSlot(el);
    if (!slot) continue;
    next.add(id);
    const state = data?.rows.find((r) => r.item === key) ?? null;
    const chip = state ? chipOf(state, ws, true) : null;
    const target: Target = { repo: item!.repo, number: item!.number, kind: item!.kind, title: item!.title };
    const w = mount(id, JSON.stringify([chip?.text, chip?.tone, ws, state?.folder]), slot.el, slot.place);
    w.host.setAttribute("data-notification", "");
    hoverWith(el, w);
    if (!w.fresh) continue;
    const add = (b: HTMLElement) => addPopover(w.root, b, item!.repo, () => notificationsStale());
    if (chip) w.root.append(chipButton(chip, add));
    if (!ws) continue;
    const review = item!.kind === "pr";
    const inFolder = !!state?.folder;
    const ask = (anchor: HTMLElement, kind: "review" | "ask" | "reply") => inFolder || !state
      ? targetPopover(w.root, anchor, { targets: [target], kind, where: d?.workspace?.name, done: () => st.stale() })
      : askWithoutCloning(w.root, anchor, null, target, () => st.stale());
    const act: HTMLButtonElement = button(review ? "Review with agent" : "Ask", () => ask(act, review ? "review" : "ask"), { small: true, title: review ? "A suggested review for you to post, without opening the thread" : "Ask a bot about this issue, without opening the thread" });
    const menu = more(w.root, "Launchpad", () => [
      { label: "Ask a bot…", hint: "This item as context", onSelect: () => ask(act, "ask") },
      { label: "Draft a reply", hint: "A suggested comment for you to post", onSelect: () => ask(act, "reply") },
      ...(inFolder ? [{ label: "Open in Launchpad", hint: "This item in Launchpad", onSelect: () => open(`/github/${item!.repo}/${item!.number}`, ws) }] : [{ label: "Add to Launchpad", hint: "Clone from GitHub into a workspace", onSelect: () => add(act) }]),
    ]);
    w.root.append(h("span", { class: "lp-hover" }, act, menu));
  }
  for (const id of shown) if (!next.has(id)) unmount(id);
  shown = next;
}

// N3: GitHub's own Done, mark as read and Unsubscribe (a row's buttons, the bulk actions above the list, and the
// keyboard shortcuts that submit them) are observed when their form is submitted, then confirmed in the page (the row
// left the list, or lost its unread mark) before Launchpad records them. Nothing here clicks or submits.

const FORMS: Record<string, ObservedAction> = { archive: "done", mark_as_read: "read", unsubscribe: "unsubscribe" };

function actionOf(form: HTMLFormElement): ObservedAction | null {
  try {
    const path = new URL(form.getAttribute("action") ?? "", location.origin).pathname;
    const m = path.match(/^\/notifications\/(?:beta\/)?(archive|mark_as_read|unsubscribe)$/);
    return m ? FORMS[m[1]] : null;
  } catch { return null; }
}

/** The rows a form acts on: the IDs it submits, else the row it sits in, else the rows checked in the list. */
function rowsOf(form: HTMLFormElement): Element[] {
  const ids = new FormData(form).getAll("notification_ids[]").map(String).filter(Boolean);
  const byId = ids.map((id) => document.querySelector(`[data-notification-id="${CSS.escape(id)}"]`)).filter((x): x is Element => !!x);
  if (byId.length) return byId;
  const own = form.closest("li[data-notification-id], .notifications-list-item");
  if (own) return [own];
  return Array.from(document.querySelectorAll("li[data-notification-id], .notifications-list-item")).filter((li) => !!li.querySelector('input[type="checkbox"]:checked'));
}

/** GitHub applied it: the row left the list or is hidden (Done, Unsubscribe), or lost its unread mark (read). */
function applied(row: Element, action: ObservedAction) {
  if (!row.isConnected || (row as HTMLElement).hidden || row.getClientRects().length === 0) return true;
  return action === "read" && !row.classList.contains("notification-unread");
}

function observe(rows: Element[], action: ObservedAction) {
  const items = rows.map((r) => rowItem(r)).filter((x) => !!x).map((x) => `${x!.repo}#${x!.number}`);
  if (!items.length) return;
  const started = Date.now();
  const check = () => {
    if (rows.every((r) => applied(r, action))) {
      void request("inbox.observed", { items, action }).then(() => notificationsStale()).catch((e) => console.warn("Launchpad: observed", e));
      return;
    }
    if (Date.now() - started < 8_000) setTimeout(check, 400);
  };
  setTimeout(check, 300);
}

let watching = false;
/** Listens (capture phase, never preventing anything) for the submits of GitHub's notification forms. */
export function watchNotificationForms(isNotifications: () => boolean) {
  if (watching) return;
  watching = true;
  document.addEventListener("submit", (e) => {
    if (!isNotifications() || !(e.target instanceof HTMLFormElement)) return;
    const action = actionOf(e.target);
    if (action) observe(rowsOf(e.target), action);
  }, true);
}
