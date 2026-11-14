// The private timeline of a pull request or issue, layered into GitHub's own timeline: the workspace's private notes
// (`launchpad note`, the owner's `note.write`) and the suggested comments that came without a note, each at its moment
// between GitHub's timeline items (`timeline-items` anchor, `timeOf`), looking like a comment from a Launchpad agent.
// Only the owner sees them: they live on Home and are drawn in closed shadow roots, never written into GitHub's page or
// fields. A note's suggested comment shows nested, with Post this comment (Launchpad's `suggestion.post`, after a
// confirmation in the entry, on a trusted click only), Edit (`suggestion.edit`), Insert into comment box (insert.ts)
// and Discard. Above GitHub's comment form, the owner's box adds a private note and tells the agents about it.
// Without timeline items, the entries go above the comment form, else under the description: nothing is lost.

import type { ExtContext } from "@/gen/ExtContext";
import type { ExtNote } from "@/gen/ExtNote";
import type { ExtSuggestion } from "@/gen/ExtSuggestion";
import { request, sentSummary } from "../../shared/messages";
import { openAgentView } from "./agentView";
import { findAnchor, findAnchors, timeOf } from "./anchors";
import { insertSuggestion, NeedsChoice } from "./insert";
import { agentPath, elapsed, open, running, type ItemPage } from "./item";
import { markdown } from "./markdown";
import { button, h, logo, more, mount, popover, unmount } from "./widgets";

/** One entry: a note, or a suggested comment without a note. */
type Entry = { key: string; at: number; note: ExtNote | null; suggestion: ExtSuggestion | null };

const PREFIX = "tl-";
const BOX = "note-box";
/** Entries unfolded in this tab. */
const unfolded = new Set<string>();
/** Suggestions being edited: their text so far. */
const editing = new Map<string, string>();
/** Suggestions whose Post asks for confirmation. */
const confirming = new Set<string>();
/** Errors of an entry's last action. */
const errors = new Map<string, string>();
/** The owner's note being typed, per page, and the last answer of Add private note. */
const drafts = new Map<string, string>();
let sent: { url: string; text: string; error: boolean } | null = null;

const WAITING = ["draft", "inserted", "posting", "uncertain"];
const ago = (at: number) => { const e = elapsed(at); return e === "now" ? "now" : `${e} ago`; };
const fail = (e: unknown) => (e instanceof Error ? e.message : String(e));
/** A text field of ours: GitHub's keyboard shortcuts (listening on the document) never see its keys. */
function field(label: string, value: string, placeholder?: string) {
  const area = h("textarea", { attrs: { "aria-label": label, ...(placeholder ? { placeholder } : {}) } });
  area.value = value;
  for (const type of ["keydown", "keypress", "keyup"] as const) area.addEventListener(type, (e) => e.stopPropagation());
  return area;
}

/** Pure: the entries in time order (notes, then the waiting suggested comments of this item no note carries). */
function entriesOf(ctx: ExtContext, number: number): Entry[] {
  const carried = new Set(ctx.notes.map((n) => n.suggestion?.id).filter((x): x is string => !!x));
  const out: Entry[] = ctx.notes.map((n) => ({ key: n.id, at: n.created_at, note: n, suggestion: n.suggestion ? ctx.suggestions.find((s) => s.id === n.suggestion!.id) ?? null : null }));
  for (const s of ctx.suggestions) {
    if (s.kind !== "comment" || s.number !== number || carried.has(s.id) || !WAITING.includes(s.status)) continue;
    out.push({ key: s.id, at: s.created_at, note: null, suggestion: s });
  }
  return out.sort((a, b) => a.at - b.at);
}

/** Pure: where each entry goes among timeline items of known times (page order): before the first later item, else
 * after the last one (`-1`). Consecutive entries with the same place form one group. */
function placeEntries(items: (number | null)[], at: number[]): { before: number; entries: number[] }[] {
  const groups: { before: number; entries: number[] }[] = [];
  at.forEach((t, i) => {
    const before = items.findIndex((x) => x !== null && x > t);
    const last = groups.at(-1);
    if (last && last.before === before) last.entries.push(i); else groups.push({ before, entries: [i] });
  });
  return groups;
}

export function renderTimeline(page: ItemPage, ctx: ExtContext | null, refresh: () => void) {
  const ws = ctx ? ctx.folder?.workspace_id ?? ctx.workspace?.id ?? null : null;
  const form = findAnchor("comment-form", { repo: page.repo, number: page.number });
  const box = ctx && ws && form ? renderBox(page, ctx, ws, form.el, refresh) : (unmount(BOX), null);
  const entries = ctx ? entriesOf(ctx, page.number) : [];
  const ids = new Set<string>();
  if (ctx && entries.length) {
    const found = findAnchors("timeline-items", { repo: page.repo, number: page.number });
    const items = found?.els ?? [];
    const groups: { anchor: Element; place: InsertPosition; entries: Entry[] }[] = [];
    if (items.length) {
      for (const g of placeEntries(items.map(timeOf), entries.map((e) => e.at))) {
        const list = g.entries.map((i) => entries[i]!);
        groups.push(g.before >= 0 ? { anchor: items[g.before]!, place: "beforebegin", entries: list } : { anchor: items.at(-1)!, place: "afterend", entries: list });
      }
    } else if (form) {
      groups.push({ anchor: box ?? form.el, place: "beforebegin", entries });
    } else {
      const d = findAnchor("item-description", { repo: page.repo, number: page.number });
      if (d) groups.push({ anchor: d.el, place: d.place, entries });
    }
    for (const g of groups) {
      const id = `${PREFIX}${g.entries[0]!.key}`;
      ids.add(id);
      const key = JSON.stringify([page.url, ctx.page.head, g.entries.map((e) => [e.key, e.note?.body.length, e.note?.suggestion?.status, e.suggestion?.status, e.suggestion?.body, editing.has(e.suggestion?.id ?? ""), confirming.has(e.suggestion?.id ?? ""), errors.get(e.key), unfolded.has(e.key)])]);
      const w = mount(id, key, g.anchor, g.place, "card");
      if (!w.fresh) continue;
      for (const e of g.entries) w.root.append(entry(w.root, page, ctx, ws, e, refresh));
    }
  }
  for (const el of document.querySelectorAll<HTMLElement>(`[data-launchpad^="${PREFIX}"]`)) if (!ids.has(el.dataset.launchpad ?? "")) el.remove();
}

export function clearTimeline() {
  unmount(BOX);
  for (const el of document.querySelectorAll<HTMLElement>(`[data-launchpad^="${PREFIX}"]`)) el.remove();
}

/** One entry, like a GitHub comment: the logo as avatar, a header (author · private note · age), the body. */
function entry(root: ShadowRoot, page: ItemPage, ctx: ExtContext, ws: string | null, e: Entry, refresh: () => void) {
  const n = e.note;
  const sessionId = n?.session_id ?? e.suggestion?.session_id ?? null;
  const agent = sessionId ? ctx.agents.find((a) => a.session_id === sessionId) ?? null : null;
  const author = n ? n.author : e.suggestion!.author;
  const what = n ? "private note" : "suggests a comment";
  const head = h("div", { class: "lp-tl-head" }, h("span", { class: "lp-grow" }, h("strong", { text: author }), h("span", { class: "lp-muted", text: ` · ${what} · ${ago(e.at)}` })));
  if (n?.reply_to) {
    const to = ctx.notes.find((x) => x.id === n.reply_to);
    if (to) head.append(h("span", { class: "lp-tag", text: `answers ${to.by_owner ? "your note" : to.author}` }));
  }
  const written = n?.head ?? e.suggestion?.head ?? null;
  if (written && ctx.page.head && written !== ctx.page.head) head.append(h("span", { class: "lp-tag", text: `head ${written.slice(0, 7)}`, title: `Written when the pull request's head was ${written.slice(0, 7)}; it is now at ${ctx.page.head.slice(0, 7)}` }));
  if (sessionId) head.append(button("Show work", () => openAgentView({ session_id: sessionId, title: agent?.title, bot: agent?.bot ?? author, running: agent ? running(agent) : undefined }, ws), { small: true, title: "What it was asked, its answers and its steps, here on the page" }));
  const menu: HTMLElement = more(root, n ? "Note" : "Suggested comment", () => [
    ...(sessionId ? [{ label: "Open the agent in Launchpad", hint: "Its full trace and thread", onSelect: () => open(agentPath({ session_id: sessionId }), ws) }] : []),
    ...(n ? [{ label: "Delete note…", hint: "Removes it from Launchpad", onSelect: () => confirmDelete(root, menu, n, refresh) }] : []),
    ...(!n && e.suggestion ? [{ label: "Discard…", hint: "Deletes the suggested comment", onSelect: () => confirmDiscard(root, menu, e.suggestion!, refresh) }] : []),
  ]);
  head.append(menu);
  const body = h("div", { class: "lp-tl-body" });
  if (n) {
    const long = n.body.length > 1500 && !unfolded.has(e.key);
    const md = markdown(n.body);
    if (long) md.setAttribute("style", "max-height: 22em; overflow: hidden; -webkit-mask-image: linear-gradient(180deg, #000 75%, transparent); mask-image: linear-gradient(180deg, #000 75%, transparent)");
    body.append(md);
    if (long) body.append(h("div", { class: "lp-actions" }, button("Show all", () => { unfolded.add(e.key); refresh(); }, { small: true })));
  }
  const err = errors.get(e.key);
  if (err) body.append(h("div", { class: "lp-error", text: err, attrs: { role: "alert" } }));
  const card = h("div", { class: "lp-tl-box" }, head, body);
  const sugg = suggestionBlock(root, page, e, refresh);
  if (sugg) (n ? card : body).append(sugg);
  return h("div", { class: "lp-tl" }, h("div", { class: "lp-tl-avatar", attrs: { "aria-hidden": "true" } }, logo()), card);
}

/** The suggested comment of an entry: waiting (Post this comment, Edit, Insert, Discard) or its one-line state. */
function suggestionBlock(root: ShadowRoot, page: ItemPage, e: Entry, refresh: () => void): HTMLElement | null {
  const s = e.suggestion;
  const state = e.note?.suggestion ?? null;
  if (!s) {
    if (!state) return null;
    const line = h("div", { class: "lp-sugg lp-row" }, logo());
    if (state.status === "posted" || state.status === "posted_by_hand") {
      line.append(h("span", { class: "lp-grow", text: state.status === "posted" ? "Suggested comment posted" : "Suggested comment posted by you on GitHub" }));
      if (state.url?.startsWith("https://github.com/")) line.append(h("a", { class: "lp-plain", text: "View", attrs: { href: state.url } }));
    } else line.append(h("span", { class: "lp-grow lp-muted", text: state.status === "discarded" ? "Suggested comment discarded" : `Suggested comment: ${state.status.replace(/_/g, " ")}` }));
    return line;
  }
  const wrap = h("div", { class: "lp-sugg" });
  const to = s.reply_to ? ` · reply to ${s.reply_to.author ?? "a comment"}` : "";
  const status = s.status === "inserted" ? " · inserted into GitHub's box" : s.status === "posting" ? " · posting…" : s.status === "uncertain" ? " · posting may have failed: check GitHub" : "";
  if (e.note) wrap.append(h("div", { class: "lp-row" }, logo(), h("strong", { class: "lp-grow", text: `Suggested comment${to}${status}` })));
  else if (to || status) wrap.append(h("div", { class: "lp-muted", text: `${to || " "}${status}`.replace(/^ · /, "").trim() }));
  const draft = editing.get(s.id);
  if (draft !== undefined) {
    const area = field("Suggested comment", draft);
    area.addEventListener("input", () => editing.set(s.id, area.value));
    const save = button("Save", () => {
      save.disabled = true;
      request("suggestion.edit", { id: s.id, body: area.value }).then(() => { editing.delete(s.id); errors.delete(e.key); refresh(); }).catch((x) => { errors.set(e.key, fail(x)); save.disabled = false; refresh(); });
    }, { small: true, primary: true });
    wrap.append(area, h("div", { class: "lp-actions" }, save, button("Cancel", () => { editing.delete(s.id); refresh(); }, { small: true })));
    return wrap;
  }
  wrap.append(h("div", { attrs: { style: "margin-top: 6px" } }, markdown(s.text)));
  if (s.status !== "draft" && s.status !== "inserted") return wrap;
  if (confirming.has(s.id)) {
    const go = h("button", { class: "lp-btn lp-primary lp-small", text: "Post", attrs: { type: "button" } });
    go.addEventListener("click", (ev) => {
      ev.preventDefault();
      ev.stopPropagation();
      // Only a real click of the owner posts (never a click the page synthesized).
      if (!ev.isTrusted || go.disabled) return;
      go.disabled = true;
      go.textContent = "Posting…";
      request("suggestion.post", { id: s.id }).then(() => { confirming.delete(s.id); errors.delete(e.key); refresh(); }).catch((x) => { errors.set(e.key, fail(x)); confirming.delete(s.id); refresh(); });
    });
    wrap.append(h("div", { class: "lp-confirm", attrs: { style: "margin-top: 8px" }, text: `Launchpad posts this text to ${page.repo}#${page.number} as you${s.reply_to?.kind === "review_comment" ? ", in the review thread it answers" : ""}, with the attribution line.` }),
      h("div", { class: "lp-actions" }, go, button("Cancel", () => { confirming.delete(s.id); refresh(); }, { small: true })));
    return wrap;
  }
  const actions = h("div", { class: "lp-actions" });
  if (s.status === "draft") {
    actions.append(button("Post this comment", () => { confirming.add(s.id); refresh(); }, { small: true, primary: true, title: "Launchpad posts it as you, after you confirm" }));
    actions.append(button("Edit", () => { editing.set(s.id, s.body); refresh(); }, { small: true, title: "Change the text before posting it" }));
  }
  const choice = h("div", { class: "lp-actions", attrs: { hidden: "" } });
  const insert = (mode?: "replace" | "append") => {
    choice.hidden = true;
    insertSuggestion(page, s, mode).catch((x) => {
      if (x instanceof NeedsChoice) {
        choice.replaceChildren(h("span", { class: "lp-muted lp-grow", text: "GitHub's box already has text." }), button("Replace it", () => insert("replace"), { small: true }), button("Add below", () => insert("append"), { small: true }), button("Cancel", () => { choice.hidden = true; }, { small: true }));
        choice.hidden = false;
      } else { errors.set(e.key, fail(x)); refresh(); }
    });
  };
  actions.append(button(s.status === "inserted" ? "Insert again" : "Insert into comment box", () => insert(), { small: true, title: "Puts the text into GitHub's box; you edit it and post it with GitHub's button" }));
  const discard: HTMLElement = button("Discard…", () => confirmDiscard(root, discard, s, refresh), { small: true });
  actions.append(discard);
  wrap.append(actions, choice);
  return wrap;
}

/** The owner's private note box, right above GitHub's comment form. Returns its host. */
function renderBox(page: ItemPage, ctx: ExtContext, ws: string, form: Element, refresh: () => void): HTMLElement {
  const last = sent?.url === page.url ? sent : null;
  const w = mount(BOX, JSON.stringify([page.url, ws, last]), form, "beforebegin", "card");
  if (!w.fresh) return w.host;
  const area = field("Private note to your agents", drafts.get(page.url) ?? "", "A question or a thought about this item for your agents. Only Launchpad keeps it; nothing goes to GitHub.");
  area.addEventListener("input", () => drafts.set(page.url, area.value));
  const add = h("button", { class: "lp-btn lp-primary lp-small", text: "Add private note", attrs: { type: "button" } });
  add.addEventListener("click", (ev) => {
    ev.preventDefault();
    ev.stopPropagation();
    if (!ev.isTrusted || add.disabled || !area.value.trim()) return;
    add.disabled = true;
    add.textContent = "Adding…";
    request("note.write", { repo: page.repo, number: page.number, body: area.value, workspace: ws })
      .then((r) => {
        drafts.delete(page.url);
        sent = { url: page.url, ...sentSummary(r, "Saved. No agent of this workspace wrote about this item and it has no default bot.") };
        refresh();
      })
      .catch((x) => { sent = { url: page.url, text: fail(x), error: true }; refresh(); });
  });
  const others = ctx.notes.some((n) => !n.by_owner) || ctx.suggestions.some((s) => s.number === page.number);
  w.root.append(h("div", { class: "lp-card lp-notebox" },
    h("div", { class: "lp-row" }, logo(), h("span", { class: "lp-title", text: "Private note to your agents" }), h("span", { class: "lp-muted lp-grow", text: others ? "· the agents that wrote about this item answer if useful" : `· ${ctx.defaults.bot ? `${ctx.defaults.bot.name} answers if useful` : "stored in Launchpad only"}` })),
    area,
    h("div", { class: "lp-actions" }, add, last ? h("span", { class: last.error ? "lp-error lp-grow" : "lp-muted lp-grow", text: last.text, attrs: { role: "status", style: "margin-top: 0" } }) : null)));
  return w.host;
}

function confirmDelete(root: ShadowRoot, anchor: HTMLElement, n: ExtNote, refresh: () => void) {
  const error = h("div", { class: "lp-error", attrs: { role: "alert", hidden: "" } });
  popover(root, anchor, "Delete note", (close) => [
    h("div", { text: n.by_owner ? "Deletes your private note from Launchpad. Agents already told about it keep their message." : `Deletes ${n.author}'s note from this item's private timeline.` }),
    error,
    h("div", { class: "lp-row" }, h("span", { class: "lp-grow" }), button("Cancel", close, { small: true }), button("Delete", () => {
      request("note.delete", { id: n.id }).then(() => { close(); refresh(); }).catch((e) => { error.textContent = fail(e); error.hidden = false; });
    }, { small: true, primary: true })),
  ]);
}

function confirmDiscard(root: ShadowRoot, anchor: HTMLElement, s: ExtSuggestion, refresh: () => void) {
  const error = h("div", { class: "lp-error", attrs: { role: "alert", hidden: "" } });
  popover(root, anchor, "Discard suggested comment", (close) => [
    h("div", { text: `Deletes ${s.author}'s suggested comment from Launchpad. Nothing was posted.` }),
    error,
    h("div", { class: "lp-row" }, h("span", { class: "lp-grow" }), button("Cancel", close, { small: true }), button("Discard", () => {
      request("suggestion.discard", { id: s.id }).then(() => { close(); refresh(); }).catch((e) => { error.textContent = fail(e); error.hidden = false; });
    }, { small: true, primary: true })),
  ]);
}
