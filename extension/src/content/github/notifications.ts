// github.com/notifications: on each row's repository line ("owner/repo #123"), so rows keep GitHub's height, the row
// chip (an agent asks, works, finished, a review or reply ready…), the notes tag (teal "2 new notes" while the
// agents' private notes about the item are unread, quiet "2 notes" after), and on hover Review with agent (pull
// requests) or Ask (issues), with ⋯ for Ask a bot, Draft a reply and Add to Launchpad. Each popover takes a note for
// the bot, so the agent starts on the item with the owner's own message. Launchpad reads GitHub's list only to find
// each row's item (from its link); it never marks, archives or clicks anything there.

import { addPopover, askWithoutCloning } from "./addRepo";
import { findAnchors, notificationSlot, rowItem } from "./anchors";
import type { Page } from "./page";
import { chipButton, chipOf, hoverWith, notesTag, RowStates } from "./rows";
import { targetPopover, type Target } from "./target";
import { button, h, more, mount, unmount } from "./widgets";

let states: RowStates | null = null;
let shown = new Set<string>();

export function notificationsStale() {
  states?.stale();
}

export function clearNotifications() {
  for (const id of shown) unmount(id);
  shown = new Set();
  states?.clear();
}

export function renderNotifications(page: Page, rerender: () => void) {
  if (page.kind !== "notifications") return clearNotifications();
  states ??= new RowStates(rerender);
  const st = states;
  const found = findAnchors("notification-rows", { repo: "", number: null });
  const rows = (found?.els ?? []).map((el) => ({ el, item: rowItem(el) })).filter((r) => r.item);
  const data = rows.length ? st.get(page.url, rows.map((r) => `${r.item!.repo}#${r.item!.number}`)) : null;
  const ws = data?.workspace ?? null;
  const next = new Set<string>();
  for (const { el, item } of rows) {
    const key = `${item!.repo}#${item!.number}`;
    const slot = notificationSlot(el);
    if (!slot) continue;
    const id = `nrow-${key.replace(/[^A-Za-z0-9]/g, "-")}`;
    next.add(id);
    const state = data?.rows.find((r) => r.item === key) ?? null;
    const chip = state ? chipOf(state, ws?.id ?? null) : null;
    const target: Target = { repo: item!.repo, number: item!.number, kind: item!.kind, title: item!.title };
    const w = mount(id, JSON.stringify([chip?.text, chip?.tone, state?.notes.total, state?.notes.unread, ws?.id, state?.folder]), slot.el, slot.place);
    w.host.setAttribute("data-notification", "");
    hoverWith(el, w);
    if (!w.fresh) continue;
    if (chip) w.root.append(chipButton(chip));
    const tag = state ? notesTag(state, item!.url) : null;
    if (tag) w.root.append(tag);
    if (!ws || !state) continue;
    const inFolder = !!state.folder;
    const done = () => st.stale();
    const ask = (anchor: HTMLElement, kind: "review" | "ask" | "reply") => inFolder || kind !== "ask"
      ? targetPopover(w.root, anchor, { targets: [target], kind, where: inFolder ? ws.name : `${ws.name} · no clone`, done })
      : askWithoutCloning(w.root, anchor, null, target, done);
    const review = item!.kind === "pr";
    const act: HTMLButtonElement = button(review ? "Review with agent" : "Ask", () => ask(act, review ? "review" : "ask"), { small: true, title: review ? "A bot reviews it and drafts a review for you to post: add your note in the popover" : "Ask a bot about this issue: write your question in the popover" });
    const menu = more(w.root, "Launchpad", () => [
      { label: "Ask a bot…", hint: "Your question, with this item as context", onSelect: () => ask(act, "ask") },
      { label: "Draft a reply…", hint: "A suggested comment for you to post", onSelect: () => ask(act, "reply") },
      ...(inFolder ? [] : [{ separator: true } as const, { label: "Add to Launchpad", hint: `Clone ${item!.repo} into a workspace`, onSelect: () => addPopover(w.root, act, item!.repo, done) }]),
    ]);
    w.root.append(h("span", { class: "lp-hover" }, act, menu));
  }
  for (const id of shown) if (!next.has(id)) unmount(id);
  shown = next;
}
