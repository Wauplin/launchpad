// The private timeline of a pull request or issue, layered into GitHub's own timeline: the workspace's private notes
// (`launchpad note`, the owner's `note.write`) and the suggested comments that came without a note, each at its moment
// between GitHub's timeline items (`timeline-items` anchor, `timeOf`), looking like a comment from a Launchpad agent.
// Only the owner sees them: they live on Home and are drawn in closed shadow roots, never written into GitHub's page or
// fields. A suggested comment shows as its own section at the bottom of the entry, with Post this comment
// (Launchpad's `suggestion.post`, in one trusted click, with the agent's actions ticked under it: close, reopen,
// request reviewers, ready for review, draft, update branch; the owner unticks some, and the button names what the
// click does: Comment and close, Approve and merge…; an action-only suggestion has no text), Edit (`suggestion.edit`: a comment's text, a review's summary and each line comment, raw Markdown), Insert into comment box (insert.ts)
// and Discard. Above GitHub's comment form, the owner's box adds a private note and tells the agents about it.
// Without timeline items, the entries go above the comment form, else under the description: nothing is lost.
// Notes link to other notes with their link (`#lp-note-<id>`, refs.ts): a link to a note of this item scrolls to it and
// marks it (also when the page opens with that fragment), a link to another item's note opens that page; each note
// lists the notes of this item that reference it, and Copy link gives its own link.

import type { ExtContext } from "@/gen/ExtContext";
import type { ExtNote } from "@/gen/ExtNote";
import type { ExtSuggestion } from "@/gen/ExtSuggestion";
import { editRequest, edited, lineKeys, lineText, startEdit, submitSuggestion, type EditDraft } from "./suggestionAction";
import type { ItemAction } from "@/gen/ItemAction";
import { describeAction, postButton, ranked } from "@/lib/suggestionActions";
import { request, sentSummary } from "../../shared/messages";
import { openAgentView } from "./agentView";
import { findAnchor, findAnchors, timeOf } from "./anchors";
import { insertSuggestion, NeedsChoice } from "./insert";
import { agentPath, elapsed, open, running, type ItemPage } from "./item";
import { markdown, type MdOptions } from "./markdown";
import { noteIdsIn, type NoteRef } from "./refs";
import { button, h, logo, more, mount, popover, unmount } from "./widgets";

/** One entry: a note, or a suggested comment without a note. */
type Entry = { key: string; at: number; note: ExtNote | null; suggestion: ExtSuggestion | null };

const PREFIX = "tl-";
const BOX = "note-box";
/** Entries unfolded in this tab. */
const unfolded = new Set<string>();
/** Suggestions being edited: their text so far (a review's summary and line comments), and those being saved. */
const editing = new Map<string, EditDraft>();
const saving = new Set<string>();
/** Errors of an entry's last action. */
const errors = new Map<string, string>();
const pending = new Set<string>();
/** The actions the owner unticked or ticked, per suggestion (kinds); unset: the agent's own. */
const ticks = new Map<string, Set<string>>();
/** The owner's note being typed, per page, and the last answer of Add private note. */
const drafts = new Map<string, string>();
let sent: { url: string; text: string; error: boolean } | null = null;
/** The entry of each note on this page, the item shown, the note marked as the target and the fragment already shown. */
const noteEls = new Map<string, HTMLElement>();
let shownItem: { repo: string; number: number } | null = null;
let target: string | null = null;
let shownHash: string | null = null;

/** Notes that were unread when this tab showed them: they keep a New tag while the tab stays on the page. */
const fresh = new Set<string>();
/** The pending `note.read` of the page (its item and unread notes), and its timer. */
let reading: { key: string; timer: ReturnType<typeof setInterval> } | null = null;

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
    if (s.number !== number || carried.has(s.id) || !WAITING.includes(s.status)) continue;
    out.push({ key: s.id, at: s.created_at, note: null, suggestion: s });
  }
  return out.sort((a, b) => a.at - b.at);
}

/** A note's link (`notes::link` on Home). */
const noteLink = (repo: string, number: number, id: string) => `https://github.com/${repo}/issues/${number}#lp-note-${id}`;
const sameItem = (a: { repo: string; number: number } | null, b: { repo: string; number: number }) => !!a && a.number === b.number && a.repo.toLowerCase() === b.repo.toLowerCase();

/** A link to a note: on this item's page, scrolls to it and marks it (true); another item's note is false (the link
 * opens its page). A note of this item that is gone does nothing. */
export function showNote(note: NoteRef): boolean {
  if (!sameItem(shownItem, note)) return false;
  const el = noteEls.get(note.id);
  if (!el?.isConnected) return true;
  for (const other of noteEls.values()) other.classList.remove("lp-target");
  target = note.id;
  el.classList.add("lp-target");
  el.scrollIntoView({ block: "center", behavior: "smooth" });
  return true;
}

/** How Markdown on this item shows note links: by their author when the note is on this page. */
function mdOptions(page: ItemPage, ctx: ExtContext): MdOptions {
  return {
    here: { repo: page.repo, number: page.number },
    noteLabel: (note) => {
      if (!sameItem(page, note)) return null;
      const n = ctx.notes.find((x) => x.id === note.id);
      return n ? (n.by_owner ? "your note" : `${n.author}'s note`) : "a deleted note";
    },
    onNote: showNote,
  };
}

/** Pure: the notes that link to each note of the item (oldest first), for "Referenced in". */
function referencesOf(ctx: ExtContext, page: ItemPage): Map<string, ExtNote[]> {
  const out = new Map<string, ExtNote[]>();
  for (const n of ctx.notes) {
    for (const id of noteIdsIn(n.body, page)) {
      if (id === n.id || !ctx.notes.some((x) => x.id === id)) continue;
      out.set(id, [...(out.get(id) ?? []), n]);
    }
  }
  return out;
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

/** The agents' unread notes of the page count as read once the page was visible in a focused window for 1.5 s (the
 * same rule as Launchpad's seen sessions): `note.read`, so lists, notifications and the panel stop flagging them. */
function readSoon(page: ItemPage, ctx: ExtContext, ws: string | null) {
  const unread = ctx.notes.filter((n) => n.unread);
  for (const n of unread) fresh.add(n.id);
  if (!unread.length || !ws) return;
  const key = `${page.repo}#${page.number} ${unread.map((n) => n.id).join(",")}`;
  if (reading?.key === key) return;
  stopReading();
  let shown = 0;
  const timer = setInterval(() => {
    shown = document.visibilityState === "visible" && document.hasFocus() ? shown + 500 : 0;
    if (shown < 1500) return;
    stopReading();
    void request("note.read", { repo: page.repo, number: page.number, workspace: ws }).catch((e) => console.warn("Launchpad: note.read", e));
  }, 500);
  reading = { key, timer };
}

function stopReading() {
  if (reading) clearInterval(reading.timer);
  reading = null;
}

export function renderTimeline(page: ItemPage, ctx: ExtContext | null, refresh: () => void) {
  const ws = ctx ? ctx.folder?.workspace_id ?? ctx.workspace?.id ?? null : null;
  if (ctx) readSoon(page, ctx, ws); else stopReading();
  const form = findAnchor("comment-form", { repo: page.repo, number: page.number });
  const box = ctx && ws && form ? renderBox(page, ctx, ws, form.el, refresh) : (unmount(BOX), null);
  const entries = ctx ? entriesOf(ctx, page.number) : [];
  const ids = new Set<string>();
  if (!sameItem(shownItem, page)) { noteEls.clear(); target = null; }
  shownItem = { repo: page.repo, number: page.number };
  const refs = ctx ? referencesOf(ctx, page) : new Map<string, ExtNote[]>();
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
      const key = JSON.stringify([page.url, g.entries.map((e) => [e.key, e.note?.body.length, fresh.has(e.key), e.note?.suggestion?.status, e.suggestion?.status, e.suggestion?.body, e.suggestion?.lines, e.suggestion?.event, e.suggestion?.head, e.suggestion?.merge, e.suggestion?.run_ci, e.suggestion?.close, e.suggestion?.actions, [...(ticks.get(e.suggestion?.id ?? "") ?? [])], pending.has(e.suggestion?.id ?? ""), Object.entries(editing.get(e.suggestion?.id ?? "")?.lines ?? {}).filter(([, v]) => v === null).map(([k]) => k), editing.has(e.suggestion?.id ?? ""), saving.has(e.suggestion?.id ?? ""), errors.get(e.key), unfolded.has(e.key), refs.get(e.key)?.map((r) => r.id)]), ctx.notes.map((n) => n.id)]);
      const w = mount(id, key, g.anchor, g.place, "card");
      if (!w.fresh) continue;
      for (const e of g.entries) w.root.append(entry(w.root, page, ctx, ws, e, refs.get(e.key) ?? [], refresh));
    }
  }
  for (const el of document.querySelectorAll<HTMLElement>(`[data-launchpad^="${PREFIX}"]`)) if (!ids.has(el.dataset.launchpad ?? "")) el.remove();
  // The page opened at a note's link: show that note once it is drawn.
  const hash = location.hash.match(/^#lp-note-([A-Za-z0-9-]{1,64})$/);
  if (hash && shownHash !== location.href && noteEls.get(hash[1]!)?.isConnected) {
    shownHash = location.href;
    showNote({ ...page, id: hash[1]! });
  }
}

export function clearTimeline() {
  stopReading();
  fresh.clear();
  unmount(BOX);
  noteEls.clear();
  shownItem = null;
  for (const el of document.querySelectorAll<HTMLElement>(`[data-launchpad^="${PREFIX}"]`)) el.remove();
}

/** One entry, like a GitHub comment: the logo as avatar, a header (author · private note · age), the body. */
function entry(root: ShadowRoot, page: ItemPage, ctx: ExtContext, ws: string | null, e: Entry, refs: ExtNote[], refresh: () => void) {
  const n = e.note;
  const sessionId = n?.session_id ?? e.suggestion?.session_id ?? null;
  const agent = sessionId ? ctx.agents.find((a) => a.session_id === sessionId) ?? null : null;
  const author = n ? n.author : e.suggestion!.author;
  const what = n ? "private note" : `suggests ${suggestionLabel(e.suggestion!).toLowerCase()}`;
  const head = h("div", { class: "lp-tl-head" }, h("span", { class: "lp-grow" }, h("strong", { text: author }), h("span", { class: "lp-muted", text: ` · ${what} · ${ago(e.at)}` })));
  if (n && fresh.has(n.id)) head.append(h("span", { class: "lp-new", text: "New", title: "You had not read this note yet" }));
  if (n?.reply_to) {
    const to = ctx.notes.find((x) => x.id === n.reply_to);
    if (to) head.append(noteButton(`answers ${to.by_owner ? "your note" : `${to.author}'s note`}`, page, to.id, "lp-tag"));
  }
  if (sessionId) head.append(button("Show work", () => openAgentView({ session_id: sessionId, title: agent?.title, bot: agent?.bot ?? author, running: agent ? running(agent) : undefined }, ws), { small: true, title: "What it was asked, its answers and its steps, here on the page" }));
  const menu: HTMLElement = more(root, n ? "Note" : e.suggestion!.kind === "action" ? "Suggested action" : `Suggested ${e.suggestion!.kind}`, () => [
    ...(sessionId ? [{ label: "Open the agent in Launchpad", hint: "Its full trace and thread", onSelect: () => open(agentPath({ session_id: sessionId }), ws) }] : []),
    ...(n ? [{ label: "Copy link", hint: "Its link: put it in a note to point at this one", onSelect: () => void navigator.clipboard.writeText(noteLink(page.repo, page.number, n.id)).catch(() => undefined) }] : []),
    ...(n ? [{ label: "Delete note…", hint: "Removes it from Launchpad", onSelect: () => confirmDelete(root, menu, n, refresh) }] : []),
    ...(!n && e.suggestion ? [{ label: "Discard…", hint: "Deletes the suggested comment", onSelect: () => confirmDiscard(root, menu, e.suggestion!, refresh) }] : []),
  ]);
  head.append(menu);
  const body = h("div", { class: "lp-tl-body" });
  if (n) {
    const long = n.body.length > 1500 && !unfolded.has(e.key);
    const md = markdown(n.body, mdOptions(page, ctx));
    if (long) md.setAttribute("style", "max-height: 22em; overflow: hidden; -webkit-mask-image: linear-gradient(180deg, #000 75%, transparent); mask-image: linear-gradient(180deg, #000 75%, transparent)");
    body.append(md);
    if (long) body.append(h("div", { class: "lp-actions" }, button("Show all", () => { unfolded.add(e.key); refresh(); }, { small: true })));
    if (refs.length) {
      const line = h("div", { class: "lp-refs lp-muted" }, "Referenced in ");
      refs.forEach((r, i) => line.append(i ? ", " : "", noteButton(`${r.by_owner ? "your note" : `${r.author}'s note`} · ${ago(r.created_at)}`, page, r.id, "lp-plain")));
      body.append(line);
    }
  }
  const err = errors.get(e.key);
  if (err) body.append(h("div", { class: "lp-error", text: err, attrs: { role: "alert" } }));
  const card = h("div", { class: "lp-tl-box" }, head);
  if (body.childNodes.length) card.append(body);
  const sugg = suggestionBlock(root, page, e, refresh);
  if (sugg) card.append(sugg);
  const el = h("div", { class: n && n.id === target ? "lp-tl lp-target" : "lp-tl" }, h("div", { class: "lp-tl-avatar", attrs: { "aria-hidden": "true" } }, logo()), card);
  if (n) noteEls.set(n.id, el);
  return el;
}

/** A link to another note of this item: shows it on the page. */
function noteButton(label: string, page: ItemPage, id: string, cls: string) {
  const a = h("a", { class: cls, text: label, attrs: { href: noteLink(page.repo, page.number, id) } });
  a.addEventListener("click", (ev) => {
    if (ev.button !== 0 || ev.ctrlKey || ev.metaKey || ev.shiftKey || ev.altKey) return;
    if (showNote({ ...page, id })) ev.preventDefault();
  });
  return a;
}

/** Pure: what a suggestion recommends, in words ("a closing comment", "an approving review and merge", "close as
 * not planned"…). */
function suggestionLabel(s: ExtSuggestion): string {
  if (s.kind === "action") return s.headline.toLowerCase();
  const more = s.actions.filter((a) => a.kind !== "close").length ? ` and ${s.actions.length} more ${s.actions.length === 1 ? "action" : "actions"}` : "";
  if (s.kind === "comment") return (s.close ? "a closing comment" : s.reply_to ? "a reply" : "a comment") + more;
  const review = s.event === "APPROVE" ? "an approving review" : s.event === "REQUEST_CHANGES" ? "a review requesting changes" : "a review";
  return (s.merge ? `${review} and merge` : s.run_ci ? `${review} and CI run` : review) + more;
}

/** Pure: the text part of a suggestion's one-click button: a review names its event and follow-up ("Approve and merge
 * (squash)"); an action-only suggestion has none. */
function postLabel(s: ExtSuggestion): string {
  if (s.kind === "action") return "";
  if (s.kind === "comment") return s.reply_to ? "Post reply" : "Comment";
  const base = s.event === "APPROVE" ? s.merge ? `Approve and merge (${s.merge})` : "Approve" : s.event === "REQUEST_CHANGES" ? "Request changes" : "Post review";
  return `${base}${s.run_ci ? " and run CI" : ""}`;
}

/** Pure: the actions the owner may tick: the agent's own, plus a close for a comment without one (unticked). */
function actionOptions(s: ExtSuggestion): ItemAction[] {
  return ranked([...s.actions, ...(s.kind === "comment" && !s.close ? [{ kind: "close", reason: null, duplicate_of: null } as ItemAction] : [])]);
}

/** The actions ticked now: the owner's choice, else the agent's. */
function tickedActions(s: ExtSuggestion): ItemAction[] {
  const on = ticks.get(s.id);
  return actionOptions(s).filter((a) => on ? on.has(a.kind) : s.actions.some((b) => b.kind === a.kind));
}

/** An action in words on this page: a pull request closes without merging. */
const actionWords = (a: ItemAction, page: ItemPage) => a.kind === "close" && page.kind !== "issue" ? "close without merging" : describeAction(a);

/** The suggested comment of an entry, as its own section at the bottom of the card: a header, the comment as GitHub
 * would show it, and Post this comment (one click; Comment and close also closes the item) with Edit, Insert and Discard; or its one-line state. */
function suggestionBlock(root: ShadowRoot, page: ItemPage, e: Entry, refresh: () => void): HTMLElement | null {
  const s = e.suggestion;
  const state = e.note?.suggestion ?? null;
  if (!s) {
    if (!state) return null;
    const line = h("div", { class: "lp-sugg lp-sugg-done lp-row" }, logo());
    if (state.status === "posted" || state.status === "posted_by_hand") {
      line.append(h("span", { class: "lp-grow", text: state.status === "posted" ? "Suggested comment posted" : "Suggested comment posted by you on GitHub" }));
      if (state.url?.startsWith("https://github.com/")) line.append(h("a", { class: "lp-plain", text: "View", attrs: { href: state.url } }));
    } else line.append(h("span", { class: "lp-grow lp-muted", text: state.status === "discarded" ? "Suggested comment discarded" : `Suggested comment: ${state.status.replace(/_/g, " ")}` }));
    return line;
  }
  const wrap = h("div", { class: "lp-sugg" });
  const to = s.reply_to ? ` · reply to ${s.reply_to.author ?? "a comment"}` : "";
  const status = s.status === "inserted" ? " · inserted into GitHub's box" : s.status === "posting" ? " · posting…" : s.status === "uncertain" ? " · posting may have failed: check GitHub" : "";
  const title = s.kind === "action" ? `Suggested: ${s.headline}` : `Suggested ${suggestionLabel(s).replace(/^an? /, "")}`;
  wrap.append(h("div", { class: "lp-row" }, logo(), h("span", { class: "lp-title", text: title }), h("span", { class: "lp-muted lp-grow", text: `${to}${status}` })));
  const draft = editing.get(s.id);
  if (draft !== undefined && s.status === "draft") {
    wrap.append(editForm(e, s, draft, refresh));
    return wrap;
  }
  if (draft !== undefined) editing.delete(s.id);
  if (s.kind === "action") wrap.append(h("div", { class: "lp-sugg-text lp-muted", text: "No comment: only these actions." }));
  else wrap.append(h("div", { class: "lp-sugg-text" }, markdown(s.text, { here: { repo: page.repo, number: page.number } })));
  if (s.kind === "review") for (const line of s.lines.filter((l) => l.state !== "dismissed")) wrap.append(h("div", { class: "lp-sugg-text" }, h("strong", { text: `${line.path}:${line.line}` }), markdown(line.body)));
  if (s.status !== "draft" && s.status !== "inserted") return wrap;
  const actions = h("div", { class: "lp-actions" });
  if (s.kind !== "action" && s.status === "draft") actions.append(button("Edit", () => { editing.set(s.id, startEdit(s)); errors.delete(e.key); refresh(); }, { small: true, title: s.kind === "review" ? "Change the summary and the line comments before posting" : "Change the text before posting it" }));
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
  if (s.kind === "comment") actions.append(button(s.status === "inserted" ? "Insert again" : "Insert into comment box", () => insert(), { small: true, title: "Puts the text into GitHub's box; you edit it and post it with GitHub's button" }));
  const discard: HTMLElement = button("Discard…", () => confirmDiscard(root, discard, s, refresh), { small: true });
  actions.append(discard, h("span", { class: "lp-grow" }));
  if (s.status === "draft") {
    // Each action is ticked as the agent suggested it; the owner unticks some (or ticks Close) before the one click.
    const chosen = tickedActions(s);
    const options = actionOptions(s);
    if (options.length) {
      const row = h("div", { class: "lp-actions" }, h("span", { class: "lp-muted", text: s.kind === "action" ? "Do:" : "Then:" }));
      for (const o of options) {
        const box = h("input", { attrs: { type: "checkbox", "aria-label": actionWords(o, page) } });
        box.checked = chosen.some((a) => a.kind === o.kind);
        box.disabled = pending.has(s.id);
        box.addEventListener("change", () => {
          const on = new Set(tickedActions(s).map((a) => a.kind));
          if (box.checked) on.add(o.kind); else on.delete(o.kind);
          ticks.set(s.id, on);
          refresh();
        });
        row.append(h("label", { class: "lp-row" }, box, h("span", { text: actionWords(o, page) })));
      }
      wrap.append(row);
    }
    const where = `${page.repo}#${page.number} as you${s.reply_to?.kind === "review_comment" ? ", in the review thread it answers" : ""}, with the attribution line`;
    const label = postButton(postLabel(s), chosen).replace(/\bclose\b(?! as)/, page.kind === "issue" ? "close" : "close without merging");
    const go = h("button", { class: "lp-btn lp-primary", text: s.kind === "comment" && !chosen.length ? "Post this comment" : label, attrs: { type: "button", title: s.kind === "action" ? `Launchpad does this on ${page.repo}#${page.number} as you` : `Launchpad posts it to ${where}${chosen.length ? ", then does the ticked actions" : ""}` } });
    go.addEventListener("click", (ev: Event) => {
      ev.preventDefault();
      ev.stopPropagation();
      // Only a real click of the owner posts (never a click the page synthesized).
      if (!ev.isTrusted || go.disabled || pending.has(s.id)) return;
      const then = chosen.map((a) => actionWords(a, page)).join(", ");
      const what = s.kind === "action" ? `On ${page.repo}#${page.number}, as you: ${then}?` : s.merge ? `Post this review and merge ${page.repo}#${page.number} using ${s.merge} at reviewed commit ${s.head}?` : s.run_ci ? "Post this review and approve waiting CI runs? The pull request’s code will run in the repository’s CI." : `Post this ${s.kind} to ${page.repo}#${page.number} as you?`;
      if (!window.confirm(s.kind !== "action" && then ? `${what} Then: ${then}.` : what)) return;
      pending.add(s.id);
      go.disabled = true;
      go.textContent = s.kind === "action" ? "Working…" : s.merge ? "Posting and merging…" : "Posting…";
      submitSuggestion(s, chosen, (args) => request("suggestion.post", args)).then(() => { errors.delete(e.key); ticks.delete(s.id); }).catch((x) => { errors.set(e.key, fail(x)); window.alert(fail(x)); }).finally(() => { pending.delete(s.id); refresh(); });
    });
    go.disabled = pending.has(s.id) || (s.kind === "action" && !chosen.length);
    actions.append(go);
  }
  wrap.append(actions, choice);
  return wrap;
}

/** Pure: a line comment's place in words ("src/a.rs lines 4–6", "src/b.rs line 9 (removed)"). */
const placeOf = (l: { path: string; line: number; start_line: number | null; side: string }) =>
  `${l.path} ${l.start_line && l.start_line !== l.line ? `lines ${l.start_line}–${l.line}` : `line ${l.line}`}${l.side === "LEFT" ? " (removed)" : ""}`;

/** A text field for raw Markdown, as tall as its text (up to 24 lines). */
function rawField(label: string, value: string, onInput: (v: string) => void) {
  const area = field(label, value);
  area.classList.add("lp-raw");
  area.rows = Math.min(24, Math.max(4, value.split("\n").length + 1));
  area.spellcheck = true;
  area.addEventListener("input", () => onInput(area.value));
  return area;
}

/** Edit of a waiting suggestion, in place of its text: the comment (or the review's summary) and each line comment of
 * a review as raw Markdown (```suggestion blocks as written); a line comment can be left out. Places, the review's
 * event and follow-up and the ticked actions stay. Save stores it on Home (`suggestion.edit`), so Post sends what was
 * saved; Cancel drops the edit; an error keeps the text typed. */
function editForm(e: Entry, s: ExtSuggestion, d: EditDraft, refresh: () => void): HTMLElement {
  const busy = saving.has(s.id);
  const review = s.kind === "review";
  const form = h("div", { class: "lp-edit" });
  const body = rawField(review ? "Review summary" : "Suggested comment", d.body, (v) => { d.body = v; });
  body.disabled = busy;
  form.append(h("div", { class: "lp-edit-label", text: review ? "Summary" : "Comment" }), body);
  if (review) {
    const keys = lineKeys(s.lines);
    s.lines.forEach((l, i) => {
      if (l.state === "dismissed") return;
      const key = keys[i]!;
      const text = lineText(d, key, l);
      const head = h("div", { class: "lp-row lp-edit-label" }, h("span", { class: "lp-grow", text: placeOf(l) }));
      if (text === null) {
        head.append(button("Keep it", () => { d.lines[key] = l.body; refresh(); }, { small: true, title: "Puts this line comment back into the review" }));
        form.append(h("div", { class: "lp-edit-line lp-left-out" }, head, h("div", { class: "lp-muted", text: "Left out of the review" })));
        return;
      }
      const leave = button("Leave out", () => { d.lines[key] = null; refresh(); }, { small: true, title: "Posts the review without this line comment" });
      leave.disabled = busy;
      head.append(leave);
      const area = rawField(`Line comment on ${placeOf(l)}`, text, (v) => { d.lines[key] = v; });
      area.disabled = busy;
      form.append(h("div", { class: "lp-edit-line" }, head, area));
    });
  }
  const save: HTMLButtonElement = button(busy ? "Saving…" : "Save", () => {
    if (saving.has(s.id)) return;
    if (!edited(s, d)) { editing.delete(s.id); errors.delete(e.key); refresh(); return; }
    saving.add(s.id);
    refresh();
    request("suggestion.edit", editRequest(s, d))
      .then(() => { editing.delete(s.id); errors.delete(e.key); })
      .catch((x) => { errors.set(e.key, fail(x)); })
      .finally(() => { saving.delete(s.id); refresh(); });
  }, { small: true, primary: true, title: "Stores the edit in Launchpad; nothing goes to GitHub until you post it" });
  save.disabled = busy;
  const cancel = button("Cancel", () => { editing.delete(s.id); errors.delete(e.key); refresh(); }, { small: true, title: "Drops your changes" });
  cancel.disabled = busy;
  form.append(h("div", { class: "lp-actions" }, save, cancel, h("span", { class: "lp-muted lp-grow", text: review ? "Line comments keep their place in the diff." : "" })));
  return form;
}

/** The owner's private note box, right above GitHub's comment form. Returns its host. */
function renderBox(page: ItemPage, ctx: ExtContext, ws: string, form: Element, refresh: () => void): HTMLElement {
  const last = sent?.url === page.url ? sent : null;
  const w = mount(BOX, JSON.stringify([page.url, ws, last]), form, "beforebegin", "card");
  if (!w.fresh) return w.host;
  const area = field("Private note to your agents", drafts.get(page.url) ?? "", "A question or a thought about this item for your agents. Only Launchpad keeps it; nothing goes to GitHub.");
  area.addEventListener("input", () => drafts.set(page.url, area.value));
  const add = h("button", { class: "lp-btn lp-primary lp-small", text: "Add private note", attrs: { type: "button", title: "Ctrl+Enter in the box adds it too" } });
  const write = (ev: Event) => {
    ev.preventDefault();
    ev.stopPropagation();
    // Only the owner's own click or key adds a note (never an event the page synthesized).
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
  };
  add.addEventListener("click", write);
  area.addEventListener("keydown", (ev) => { if (ev.key === "Enter" && (ev.ctrlKey || ev.metaKey) && !ev.isComposing) write(ev); });
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
