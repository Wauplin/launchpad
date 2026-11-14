// I3 Insert into GitHub's box, and the in-page part of the feedback ledger (K1). A suggested comment shows in the item's
// private timeline (timeline.ts); a reply to a review comment also shows as a banner over that thread's reply box, and
// the conversation's comment box gets a banner once a suggestion was inserted or prefilled into it. Insert writes its text, plus
// the attribution line after a blank line, into GitHub's own field the way typing does (`insertText`, so Ctrl+Z
// removes it; the native value setter and an input event when the editor refuses), asking first when the box already
// has text. Then it only OBSERVES that form: when GitHub's own button or Ctrl+Enter submits it, the final text is read
// and sent as `suggestion.posted-by-hand`; Home confirms it from GitHub later. Nothing here ever calls click(), submit()
// or requestSubmit() on GitHub's elements, and nothing is posted by the extension.
// Prefill (off by default, Settings → GitHub): the newest suggested comment for the conversation's box goes into it on
// its own when the page shows it and the box is empty, once per tab, without moving the focus or the page. It claims
// nothing on Home until GitHub's form is submitted (then `inserted` and `posted-by-hand`), so Launchpad's Post stays
// possible meanwhile; a prefilled text left untouched is taken out again when the suggestion is discarded or posted
// from Launchpad.

import type { ExtContext } from "@/gen/ExtContext";
import type { ExtSuggestion } from "@/gen/ExtSuggestion";
import { request } from "../../shared/messages";
import { findAnchor } from "./anchors";
import { openAgentView } from "./agentView";
import { askPopover } from "./askPopover";
import { agentPath, ledger, open, type ItemPage } from "./item";
import { button, h, logo, more, mount, unmount } from "./widgets";

type Inserted = { box: HTMLTextAreaElement; at: number; posted: boolean; stop: () => void; /** Filled by prefill (no click): the text written. */ prefilled?: string };
/** Suggestions inserted in this tab: the box they went into and whether its form was submitted. */
const inserted = new Map<string, Inserted>();
const shown = new Set<string>();
/** Suggestions prefilled in this tab: never again, even when the owner emptied the box. */
const prefilledOnce = new Set<string>();
/** The suggestion was prefilled into GitHub's box and is still there, unposted. */
export const isPrefilled = (id: string) => { const e = inserted.get(id); return !!e?.prefilled && !e.posted; };
let onChange: () => void = () => undefined;
/** What re-renders the page after an insert or a post (main.ts sets it). */
export function onInserted(fn: () => void) { onChange = fn; }

/** The comment anchor a suggestion answers (`discussion_r…` for a review thread). */
const threadOf = (s: ExtSuggestion) => (s.reply_to?.kind === "review_comment" ? `discussion_r${s.reply_to.id}` : null);

/** GitHub's field for this suggestion: the thread's open reply box, or the conversation's comment box. */
function boxFor(page: ItemPage, s: ExtSuggestion): { box: HTMLTextAreaElement | null; opener: HTMLElement | null } {
  const thread = threadOf(s);
  if (thread) {
    const box = findAnchor("thread-reply", { repo: page.repo, number: page.number, comment: thread })?.el ?? null;
    const opener = findAnchor("thread-reply-opener", { repo: page.repo, number: page.number, comment: thread })?.el ?? null;
    return { box: box instanceof HTMLTextAreaElement ? box : null, opener: opener instanceof HTMLElement ? opener : null };
  }
  const box = findAnchor("comment-box", { repo: page.repo, number: page.number })?.el ?? null;
  return { box: box instanceof HTMLTextAreaElement ? box : null, opener: null };
}

export class NeedsChoice extends Error {}

/** Writes into a GitHub text field like typing: one undo step. `mode`: what to do with text already in the box. */
export function write(box: HTMLTextAreaElement, text: string, mode: "replace" | "append") {
  box.focus();
  if (mode === "replace") box.select();
  else {
    const end = box.value.length;
    box.setSelectionRange(end, end);
    if (box.value.trim()) text = `${box.value.endsWith("\n") ? "\n" : "\n\n"}${text}`;
  }
  const before = box.value;
  // `insertText` keeps GitHub's undo history and fires the input events its editor listens to.
  const ok = document.execCommand("insertText", false, text);
  if (!ok || box.value === before) {
    const expected = mode === "replace" ? text : before + text;
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set?.call(box, expected);
    box.dispatchEvent(new Event("input", { bubbles: true }));
  }
}

/** Insert (the banner's button, the box's main action, the panel's Insert). Throws `NeedsChoice` when the box has text
 * and no `mode` was chosen. */
export async function insertSuggestion(page: ItemPage, s: ExtSuggestion, mode?: "replace" | "append") {
  const { box, opener } = boxFor(page, s);
  if (!box) {
    if (opener) {
      // Never clicked: focusing shows the owner where GitHub's own Reply field is.
      opener.scrollIntoView({ block: "center" });
      opener.focus();
      throw new Error("Open this thread's reply box (GitHub's Reply… field), then Insert again.");
    }
    throw new Error("GitHub's comment box was not found on this page (are you signed in to GitHub?).");
  }
  if (box.value.trim() && !mode) throw new NeedsChoice("GitHub's box already has text.");
  box.scrollIntoView({ block: "center" });
  write(box, `${s.text.trimEnd()}\n\n${s.attribution}`, mode ?? "replace");
  inserted.get(s.id)?.stop();
  const entry: Inserted = { box, at: Date.now(), posted: false, stop: () => undefined };
  entry.stop = watchSubmit(box, entry, (text) => ledger("posted-by-hand", s.id, text));
  inserted.set(s.id, entry);
  onChange();
  await ledger("inserted", s.id, s.text);
}

/** Sets a field's value like GitHub's own editor expects (the native setter and an input event). */
function setValue(box: HTMLTextAreaElement, value: string) {
  Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set?.call(box, value);
  box.dispatchEvent(new Event("input", { bubbles: true }));
}

/** Prefill: writes the suggestion into the empty box without a click, keeping the focus and the scroll position. The
 * claim waits for GitHub's submit. */
function prefill(box: HTMLTextAreaElement, s: ExtSuggestion) {
  prefilledOnce.add(s.id);
  const text = `${s.text.trimEnd()}\n\n${s.attribution}`;
  const before = document.activeElement;
  const [x, y] = [window.scrollX, window.scrollY];
  box.focus({ preventScroll: true });
  box.select();
  if (!document.execCommand("insertText", false, text) || box.value !== text) setValue(box, text);
  if (before instanceof HTMLElement && before !== box && before !== document.body) before.focus({ preventScroll: true }); else box.blur();
  window.scrollTo(x, y);
  const entry: Inserted = { box, at: Date.now(), posted: false, stop: () => undefined, prefilled: text };
  entry.stop = watchSubmit(box, entry, async (posted) => { await ledger("inserted", s.id, s.text); await ledger("posted-by-hand", s.id, posted); });
  inserted.set(s.id, entry);
}

/** Scrolls to GitHub's box that holds a prefilled suggestion and focuses it. */
export function showPrefilled(id: string) {
  const e = inserted.get(id);
  if (!e?.box.isConnected) return;
  e.box.scrollIntoView({ block: "center" });
  e.box.focus({ preventScroll: true });
}

/** Takes an untouched prefill out of GitHub's box (Launchpad discarded or posted the suggestion). */
function unprefill(id: string) {
  const e = inserted.get(id);
  if (!e?.prefilled || e.posted) return;
  e.stop();
  inserted.delete(id);
  if (e.box.isConnected && e.box.value === e.prefilled) setValue(e.box, "");
}

/** Parts other than a comment's body filled into GitHub's fields (a line comment, a review's summary): `<id>|<part>`. */
const filledParts = new Map<string, Inserted>();
export const partState = (id: string, part: string): "in" | "posted" | null => {
  const e = filledParts.get(`${id}|${part}`);
  return e ? (e.posted ? "posted" : "in") : null;
};

/** Fills GitHub's `box` with one part of a suggestion (`line:…` or `body` of a review), plus the attribution line, and
 * observes its form like Insert does. `suggestion` null: text that is no suggestion (a bot's answer), not recorded.
 * Throws `NeedsChoice` when the box has text and no `mode` was chosen. */
export async function fillPart(box: HTMLTextAreaElement, suggestion: string | null, part: string, text: string, attribution: string, mode?: "replace" | "append") {
  if (box.value.trim() && !mode) throw new NeedsChoice("GitHub's box already has text.");
  box.scrollIntoView({ block: "center" });
  write(box, `${text.trimEnd()}\n\n${attribution}`, mode ?? "replace");
  if (!suggestion) return;
  const key = `${suggestion}|${part}`;
  filledParts.get(key)?.stop();
  const entry: Inserted = { box, at: Date.now(), posted: false, stop: () => undefined };
  entry.stop = watchSubmit(box, entry, (posted) => request("suggestion.ledger", { action: "posted-by-hand", id: suggestion, part, text: posted }));
  filledParts.set(key, entry);
  onChange();
  await request("suggestion.ledger", { action: "inserted", id: suggestion, part, text });
}

/** The form's submit, GitHub's own submit buttons and Ctrl/Cmd+Enter in the box: observed only (capture phase, so the
 * text is read before GitHub clears the box), never triggered. */
function watchSubmit(box: HTMLTextAreaElement, entry: Inserted, record: (text: string) => Promise<unknown>) {
  const composer = () => box.closest("form") ?? box.closest('[class*="Composer"], [data-testid*="composer" i], .timeline-comment') ?? box.parentElement;
  const posted = () => {
    const text = box.value;
    if (entry.posted || !text.trim()) return;
    entry.posted = true;
    stop();
    onChange();
    void record(text).catch((e) => console.warn("Launchpad: posted by hand", e));
  };
  const onSubmit = (e: Event) => { if (e.target instanceof Element && e.target.contains(box)) posted(); };
  const onClick = (e: Event) => {
    const b = (e.target as Element | null)?.closest?.("button");
    const root = composer();
    if (!b || !root || !root.contains(b) || b.disabled) return;
    const label = (b.textContent ?? "").trim();
    if (b.type === "submit" ? !/^(cancel|close|preview|write)$/i.test(label) : /^(comment|reply|close with comment|add single comment|start a review|add review comment|update comment|submit review)$/i.test(label)) posted();
  };
  const onKey = (e: KeyboardEvent) => { if (e.key === "Enter" && (e.ctrlKey || e.metaKey) && e.target === box) posted(); };
  document.addEventListener("submit", onSubmit, true);
  document.addEventListener("click", onClick, true);
  document.addEventListener("keydown", onKey, true);
  // After two hours nothing is watched any more (the record stays "inserted").
  const timer = setTimeout(() => stop(), 2 * 3600_000);
  function stop() {
    clearTimeout(timer);
    document.removeEventListener("submit", onSubmit, true);
    document.removeEventListener("click", onClick, true);
    document.removeEventListener("keydown", onKey, true);
  }
  return stop;
}

/** `suggestion.shown` once the banner (or a ghost comment: its `part`) was on screen for 1.5 s. */
export function watchShown(host: HTMLElement, id: string, part = "body") {
  const key = `${id}|${part}`;
  if (shown.has(key)) return;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const io = new IntersectionObserver(([e]) => {
    clearTimeout(timer);
    if (!e?.isIntersecting || document.visibilityState !== "visible") return;
    timer = setTimeout(() => {
      if (!host.isConnected || shown.has(key)) return;
      shown.add(key);
      io.disconnect();
      void request("suggestion.ledger", { action: "shown", id, part }).catch(() => shown.delete(key));
    }, 1_500);
  }, { threshold: 0.6 });
  io.observe(host);
}

/** The banners of the page's waiting suggested comments (idempotent); `autoFill`: prefill the conversation's box. */
export function renderInsertBanners(page: ItemPage, ctx: ExtContext | null, refresh: () => void, autoFill = true) {
  onChange = refresh;
  const list = (ctx?.suggestions ?? []).filter((s) => s.kind === "comment" && s.number === page.number && (s.status === "draft" || s.status === "inserted"));
  if (ctx) for (const id of [...inserted.keys()]) if (!list.some((s) => s.id === id)) unprefill(id);
  const ids = new Set<string>();
  // One banner over the comment box (the newest suggestion for it) and one per review thread.
  const main = list.filter((s) => !threadOf(s)).at(-1);
  if (autoFill && main && main.status === "draft" && !prefilledOnce.has(main.id) && !inserted.has(main.id)) {
    const { box } = boxFor(page, main);
    // An empty box only: never over the owner's own text or GitHub's restored draft.
    if (box && !box.value.trim()) prefill(box, main);
  }
  // The timeline shows the conversation's suggestion: its banner only follows a text in GitHub's box.
  const mainBanner = main && (autoFill || inserted.has(main.id)) ? main : undefined;
  for (const s of [mainBanner, ...list.filter(threadOf)]) {
    if (!s) continue;
    const id = `insert-${s.id}`;
    ids.add(id);
    const { box, opener } = boxFor(page, s);
    const place = box ? (box.closest("form") ?? box) : opener ? (opener.closest("form, .review-thread-reply, div") ?? opener) : null;
    if (!place) { unmount(id); continue; }
    const state = inserted.get(s.id);
    const key = `${s.id}|${s.status}|${state ? (state.posted ? "posted" : state.prefilled ? "pre" : "in") : "-"}|${!!box}`;
    const w = mount(id, key, place, "beforebegin", "banner");
    if (!w.fresh) continue;
    banner(w.root, page, ctx, s, !!box);
    watchShown(w.host, s.id);
  }
  for (const el of document.querySelectorAll<HTMLElement>('[data-launchpad^="insert-"]')) if (!ids.has(el.dataset.launchpad ?? "")) el.remove();
}

function banner(root: ShadowRoot, page: ItemPage, ctx: ExtContext | null, s: ExtSuggestion, ready: boolean) {
  const state = inserted.get(s.id);
  const ws = ctx?.folder?.workspace_id ?? ctx?.workspace?.id ?? null;
  const to = s.reply_to ? ` to ${s.reply_to.author ?? "a comment"}` : "";
  const title = state?.posted ? "Recorded: posted by you on GitHub" : state?.prefilled ? `Prefilled with ${s.bot ?? "an agent"}'s draft${to}: edit it, then post with GitHub's button` : state ? "Inserted: edit it, then post with GitHub's button" : s.status === "inserted" ? `Inserted earlier${to}` : `${s.bot ?? "An agent"} drafted a reply${to}`;
  const preview = h("div", { class: "lp-preview", text: s.text, attrs: { hidden: "" } });
  const error = h("div", { class: "lp-error", attrs: { role: "alert", hidden: "" } });
  const choice = h("div", { class: "lp-actions", attrs: { hidden: "" } });
  const row = h("div", { class: "lp-row" }, logo(), h("span", { class: "lp-grow", text: title }));
  const fail = (e: unknown) => { error.textContent = e instanceof Error ? e.message : String(e); error.hidden = false; };
  const run = (mode?: "replace" | "append") => {
    error.hidden = true;
    choice.hidden = true;
    insertSuggestion(page, s, mode).catch((e) => {
      if (e instanceof NeedsChoice) {
        choice.replaceChildren(h("span", { class: "lp-muted lp-grow", text: "GitHub's box already has text." }), button("Replace it", () => run("replace"), { small: true }), button("Add below", () => run("append"), { small: true }), button("Cancel", () => { choice.hidden = true; }, { small: true }));
        choice.hidden = false;
      } else fail(e);
    });
  };
  if (!state?.posted && !state?.prefilled) row.append(button(state || s.status === "inserted" ? "Insert again" : "Insert", () => run(), { primary: !state && s.status !== "inserted", small: true, title: ready ? "Puts the text into GitHub's box; you edit it and post it with GitHub's button" : "Open GitHub's reply box of this thread first" }));
  row.append(more(root, "Suggestion", () => {
    const agent = ctx?.agents.find((a) => a.session_id === s.session_id) ?? null;
    return [
      { label: preview.hidden ? "Preview" : "Hide preview", hint: "The suggested text", onSelect: () => { preview.hidden = !preview.hidden; } },
      ...(state?.prefilled && !state.posted ? [{ label: "Take it out of GitHub's box", hint: "Empties the box if you did not edit the text", onSelect: () => { unprefill(s.id); onChange(); } }] : []),
      ...(s.session_id ? [{ label: "Show the agent's work", hint: "Its turns and steps, here on the page", onSelect: () => openAgentView({ session_id: s.session_id!, bot: s.bot, title: agent?.title, running: agent ? ["working", "starting", "background"].includes(agent.status) : undefined }, ws) }] : []),
      { label: "Ask again with a note", hint: "The same bot, about the same comment", onSelect: () => askPopover(root, row, page, ctx, threadOf(s) ?? (s.reply_to ? `issuecomment-${s.reply_to.id}` : null), "ask", onChange, agent?.bot_id) },
      ...(s.session_id ? [{ label: "Open the agent in Launchpad", hint: "Its trace and the suggestion's card", onSelect: () => open(agentPath({ session_id: s.session_id! }), ws) }] : []),
      { note: "Discard it in Launchpad if you do not want it." },
    ];
  }));
  root.append(h("div", { class: "lp-banner" }, row, choice, preview, error));
}

/** The panel's Insert (TabCommand): the same as the banner's, without the choice (it answers with the reason). */
export async function insertFromPanel(page: ItemPage | null, ctx: ExtContext | null, id: string) {
  const s = ctx?.suggestions.find((x) => x.id === id);
  if (!page || !s) throw new Error("This suggestion is not about the page in this tab");
  try {
    await insertSuggestion(page, s);
  } catch (e) {
    if (e instanceof NeedsChoice) throw new Error("GitHub's box already has text: use Insert on the page to replace it or add below it.");
    throw e;
  }
}
