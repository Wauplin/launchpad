// A pull request's Files changed (`/pull/<n>/files`, `/changes`): D1 Ask about these lines (GitHub's own line
// selection, read from the URL fragment or its highlighted cells, sent to a bot as a reference: Home reads the lines),
// with the answer under the lines and Turn into line comment; D2 an agent's suggested review as ghost comments on
// their lines (dashed teal), each with Add to my review (fills GitHub's own line comment form) and Dismiss; D3 Insert
// summary over GitHub's Review changes dialog (its event radios and submit stay untouched); D4 ghosts of a review
// written at an older head say so, and Add needs a second click. GitHub's buttons post; nothing here clicks or
// submits GitHub's elements. The line form opens only by the owner's click on GitHub's +: a pending Add fills it then.

import type { ExtAgent } from "@/gen/ExtAgent";
import type { ExtContext } from "@/gen/ExtContext";
import type { ExtLine } from "@/gen/ExtLine";
import type { ExtSuggestion } from "@/gen/ExtSuggestion";
import type { SessionEvent } from "@/gen/SessionEvent";
import { request, type BotBrief } from "../../shared/messages";
import { findAnchor, findAnchors, findLine, hashPaths, pathOfHash, type Params } from "./anchors";
import { fillPart, NeedsChoice, partState, watchShown } from "./insert";
import { agentPath, elapsed, open, running, statusText, workspaceBots } from "./item";
import { stepOfSession, watch } from "./live";
import type { Page } from "./page";
import { button, h, logo, more, mount, popover, unmount, type MenuEntry } from "./widgets";

export type FilesPage = Page & { repo: string; number: number };
export const isFiles = (p: Page): p is FilesPage => p.kind === "pr_files" && !!p.repo && !!p.number;

type Side = "LEFT" | "RIGHT";
type Lines = { path: string; side: Side; start: number; end: number };

const short = (c: string | null | undefined) => (c ? c.slice(0, 7) : "?");
const range = (l: { start: number; end: number }) => (l.start === l.end ? `Line ${l.end}` : `Lines ${l.start}–${l.end}`);
const sideText = (s: Side) => (s === "LEFT" ? "left side" : "right side");
const fileName = (path: string) => path.split("/").at(-1) ?? path;
const message = (e: unknown) => (e instanceof Error ? e.message : String(e));
const LINK = "[Launchpad](https://github.com/Wauplin/launchpad)";

/** The lines selected the GitHub way (click a line number, shift-click another): `#diff-<sha256 of path>R12-R14`,
 * else the cells GitHub highlights. Never the page's text. */
function selected(): Lines | null {
  const m = location.hash.match(/^#diff-([0-9a-f]{64})([LR])(\d+)(?:-([LR])(\d+))?$/);
  if (m) {
    const path = pathOfHash(m[1]);
    if (!path) return null;
    const a = Number(m[3]);
    const b = m[5] ? Number(m[5]) : a;
    // A range over both sides keeps the end's side only.
    if (m[4] && m[4] !== m[2]) return { path, side: m[4] === "L" ? "LEFT" : "RIGHT", start: b, end: b };
    return { path, side: m[2] === "L" ? "LEFT" : "RIGHT", start: Math.min(a, b), end: Math.max(a, b) };
  }
  const cells = Array.from(document.querySelectorAll('td.selected-line[id^="diff-"]'));
  const ids = cells.map((c) => c.id.match(/^diff-([0-9a-f]{64})([LR])(\d+)$/)).filter((x): x is RegExpMatchArray => !!x);
  if (!ids.length || ids.some((x) => x[1] !== ids[0][1] || x[2] !== ids[0][2])) return null;
  const path = pathOfHash(ids[0][1]);
  const nums = ids.map((x) => Number(x[3]));
  return path ? { path, side: ids[0][2] === "L" ? "LEFT" : "RIGHT", start: Math.min(...nums), end: Math.max(...nums) } : null;
}

/** Questions asked from this tab about lines: their bot, session, message key and the answer once read. */
type Asked = { lines: Lines; head: string | null; bot: BotBrief; session: string; key: string; note: string; at: number; answer: string | null; readAt: number | null };
const asked = new Map<string, Asked>();
const keyOf = (l: Lines) => `${l.path}|${l.side}|${l.start}-${l.end}`;

/** Add to my review / Turn into line comment clicked while GitHub's line form was closed: filled when the owner opens it. */
type Pending = { params: Params; until: number; fill: (box: HTMLTextAreaElement) => Promise<void> };
const pending = new Map<string, Pending>();
/** Outdated ghosts the owner confirmed once (D4: the second click inserts). */
const confirmed = new Set<string>();
/** Ghosts whose last action failed: their message. */
const errors = new Map<string, string>();
/** GitHub's field already had text: the owner chooses Replace it or Add below (never decided silently). */
const choices = new Map<string, (mode: "replace" | "append") => void>();

/** The choice row of `id`, when GitHub's field had text. */
function choiceRow(id: string) {
  const choose = choices.get(id);
  if (!choose) return null;
  return h("div", { class: "lp-actions" }, h("span", { class: "lp-muted lp-grow", text: "GitHub's field already has text." }),
    button("Replace it", () => choose("replace"), { small: true }), button("Add below", () => choose("append"), { small: true }),
    button("Cancel", () => { choices.delete(id); refreshPage(); }, { small: true }));
}

/** Runs a fill; a field with text turns into the choice of `id`, an error into its message. */
function attempt(id: string, fill: (mode?: "replace" | "append") => Promise<void>, mode?: "replace" | "append") {
  errors.delete(id);
  choices.delete(id);
  return fill(mode).catch((e) => {
    if (e instanceof NeedsChoice) choices.set(id, (m) => void attempt(id, fill, m).finally(refreshPage));
    else errors.set(id, message(e));
  }).finally(refreshPage);
}

let refreshPage: () => void = () => undefined;

/** Renders every Files changed feature (idempotent). */
export function renderFiles(page: FilesPage, ctx: ExtContext | null, refresh: () => void) {
  refreshPage = refresh;
  const reviews = (ctx?.suggestions ?? []).filter((s) => s.kind === "review" && s.number === page.number && (s.status === "draft" || s.status === "inserted"));
  // GitHub's line anchors hash the path: known paths are hashed once (newer layouts carry no path attribute).
  const paths = [...reviews.flatMap((s) => s.lines.map((l) => l.path)), ...Array.from(asked.values(), (a) => a.lines.path)];
  void hashPaths(paths).catch(() => undefined);
  findAnchors("diff-files", { repo: page.repo, number: page.number });
  fillPending(page);
  renderBar(page, ctx);
  const live = renderAnswers(page, ctx);
  renderGhosts(page, ctx, reviews);
  renderReviewBanner(page, ctx, reviews);
  renderSummary(page, ctx, reviews);
  watch(live, refresh);
}

export function clearFiles() {
  for (const id of ["lines-bar", "review-banner", "review-summary"]) unmount(id);
  for (const el of document.querySelectorAll('[data-launchpad^="ghost-"], [data-launchpad^="answer-"], tr[data-launchpad-row]')) el.remove();
}

// ---- D1 Ask about these lines -------------------------------------------------------------------------------------

function renderBar(page: FilesPage, ctx: ExtContext | null) {
  const sel = ctx ? selected() : null;
  if (!sel || !ctx) return unmount("lines-bar");
  const w = mount("lines-bar", `${keyOf(sel)}|${ctx.page.head}`, document.body, "beforeend");
  if (!w.fresh) return;
  const label = `${range(sel)} · ${sideText(sel.side)} · ${fileName(sel.path)}`;
  const ask: HTMLButtonElement = button("Ask about these lines", () => askLines(w.root, ask, page, ctx, sel), { primary: true, small: true, title: "A bot of the workspace gets these lines by reference; Home reads them from the pull request" });
  w.root.append(h("div", { class: "lp-bar", attrs: { role: "region", "aria-label": "Launchpad" } }, logo(), h("span", { text: label, title: sel.path }), ask));
}

function askLines(root: ShadowRoot, anchor: HTMLElement, page: FilesPage, ctx: ExtContext, sel: Lines, prefer?: string) {
  const select = h("select", { attrs: { "aria-label": "Bot" } }, h("option", { text: "Loading bots…", attrs: { value: "" } }));
  const note = h("textarea", { attrs: { "aria-label": "Your question", placeholder: "Your question (optional): what do these lines do, is this right…" } });
  const error = h("div", { class: "lp-error", attrs: { role: "alert", hidden: "" } });
  const send = h("button", { class: "lp-btn lp-primary", text: "Send", attrs: { type: "button", disabled: "" } });
  let bots: BotBrief[] = [];
  let busy = false;
  const close = popover(root, anchor, "Ask about these lines", (close) => [
    h("div", { class: "lp-muted", text: `${range(sel)} · ${sideText(sel.side)} · ${sel.path} @ ${short(ctx.page.head)}` }),
    select, note, error,
    h("div", { class: "lp-row" }, h("span", { class: "lp-grow" }), button("Cancel", close, { small: true }), send),
  ]);
  const fail = (e: unknown) => { error.textContent = message(e); error.hidden = false; };
  const submit = async () => {
    const bot = bots.find((b) => b.id === select.value);
    if (!bot || busy) return;
    busy = true;
    send.disabled = true;
    send.textContent = "Sending…";
    const key = crypto.randomUUID();
    const question = note.value.trim();
    try {
      const text = question || "What do these lines do, and is anything wrong with them?";
      const ref = { url: `https://github.com/${page.repo}/pull/${page.number}`, kind: "pr", repo: page.repo, number: page.number, title: ctx.page.title ?? null };
      const selection = { repo: page.repo, number: page.number, path: sel.path, side: sel.side, start: sel.start, end: sel.end, head: ctx.page.head ?? null };
      const session = await request("bot.message", { bot: bot.id, text, github_refs: [ref], selection, key });
      asked.set(keyOf(sel), { lines: sel, head: ctx.page.head ?? null, bot, session: session.id, key, note: text, at: Date.now(), answer: null, readAt: null });
      close();
      refreshPage();
    } catch (e) {
      fail(e);
      busy = false;
      send.disabled = false;
      send.textContent = "Send";
    }
  };
  send.addEventListener("click", (e) => { e.preventDefault(); void submit(); });
  note.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing) { e.preventDefault(); void submit(); }
    e.stopPropagation();
  });
  void workspaceBots().then(({ bots: list, ask: target }) => {
    bots = list.filter((b) => !b.paused);
    select.replaceChildren(...bots.map((b) => h("option", { text: b.name, attrs: { value: b.id } })));
    if (!bots.length) return fail("No bot in this workspace: create one in Launchpad (Bots → New bot).");
    const first = [prefer, ctx.defaults.bot?.id, target].find((id) => id && bots.some((b) => b.id === id));
    if (first) select.value = first;
    send.disabled = false;
    note.focus();
  }).catch(fail);
}

/** The bot's answer to the question: its last message after ours (found by the message's key). */
async function readAnswer(a: Asked) {
  const page = await request("session.events", { id: a.session, tail: 200 });
  const events: SessionEvent[] = page.events;
  const at = events.findIndex((e) => e.body.kind === "user_message" && e.body.client_id === a.key);
  let answer: string | null = null;
  for (const e of at >= 0 ? events.slice(at + 1) : []) {
    if (e.body.kind === "user_message") break;
    if (e.body.kind === "assistant_message" && e.body.text.trim()) answer = e.body.text.trim();
  }
  a.answer = answer;
}

/** A host under a diff line: in a row of our own after the line's row (classic tables), else after the line. */
function underLine(row: Element, id: string, key: string, kind: "card" | "banner") {
  if (row instanceof HTMLTableRowElement) {
    let tr = document.querySelector<HTMLTableRowElement>(`tr[data-launchpad-row="${CSS.escape(id)}"]`);
    if (!tr) {
      tr = document.createElement("tr");
      tr.setAttribute("data-launchpad-row", id);
      const td = document.createElement("td");
      td.colSpan = 8;
      td.style.cssText = "padding:6px 12px 6px 60px;background:transparent;border:0;";
      tr.append(td);
    }
    // Right after the line, before GitHub's own comment rows of it.
    if (row.nextElementSibling !== tr) row.after(tr);
    return mount(id, key, tr.firstElementChild!, "beforeend", kind);
  }
  return mount(id, key, row, "afterend", kind);
}

function renderAnswers(page: FilesPage, ctx: ExtContext | null): string[] {
  const live: string[] = [];
  const ids = new Set<string>();
  for (const a of asked.values()) {
    const agent: ExtAgent | null = ctx?.agents.find((x) => x.session_id === a.session) ?? null;
    const status = agent ?? { status: "starting", since: a.at, needs_you: false, open_question: false };
    const working = running(status as ExtAgent);
    if (working) live.push(a.session);
    // Read the answer once the turn ended (again after each new turn).
    if (agent && !working && a.readAt !== agent.since) {
      a.readAt = agent.since;
      void readAnswer(a).then(refreshPage).catch(() => { a.readAt = null; });
    }
    const id = `answer-${keyOf(a.lines).replace(/[^A-Za-z0-9_-]/g, "_")}`;
    const row = findLine({ repo: page.repo, number: page.number, path: a.lines.path, side: a.lines.side, line: a.lines.end });
    if (!row) continue;
    ids.add(id);
    const step = working ? stepOfSession(a.session) : null;
    const source = lineSuggestion(ctx, a);
    const key = [status.status, step, a.answer, source?.line.state, source?.s.id, pending.has(pendingKey(a.lines)), errors.get(id), choices.has(id), working ? elapsed(status.since) : ""].join("|");
    const w = underLine(row, id, key, "card");
    if (!w.fresh) continue;
    const head = h("div", { class: "lp-row" }, logo(), h("span", { class: "lp-title", text: `${range(a.lines)} · ${sideText(a.lines.side)}` }), h("span", { class: "lp-muted lp-grow", text: `${a.bot.name} · ${working ? step || statusText(status as ExtAgent) : statusText(status as ExtAgent)}` }));
    const card = h("div", { class: "lp-card" }, head, h("div", { class: "lp-muted", text: a.note, attrs: { style: "margin-top:4px" } }));
    const ws = ctx?.folder?.workspace_id ?? ctx?.workspace?.id ?? null;
    if (a.answer) card.append(h("div", { class: "lp-answer", text: a.answer }));
    else if (!working && agent) card.append(h("div", { class: "lp-muted", text: "No answer in the chat: open it in Launchpad.", attrs: { style: "margin-top:4px" } }));
    const actions = h("div", { class: "lp-actions" });
    if (pending.has(pendingKey(a.lines))) actions.append(h("span", { class: "lp-muted lp-grow", text: `Open GitHub's comment form on line ${a.lines.end} (its +): Launchpad fills it then.` }));
    else if (a.answer || source) {
      const text = source?.line.body ?? a.answer ?? "";
      actions.append(button(source?.line.state ? "Insert again" : "Turn into line comment", () => turnIntoComment(page, a, source, text, id), { primary: !source?.line.state, small: true, title: source ? `${a.bot.name}'s suggested line comment, into GitHub's form for line ${a.lines.end}: you post it with GitHub's button` : "The answer, into GitHub's comment form of these lines: you edit and post it with GitHub's button" }));
    }
    actions.append(more(w.root, "Answer", (): MenuEntry[] => [
      { label: "Ask more", hint: `${a.bot.name}, about the same lines`, onSelect: () => askLines(w.root, head, page, ctx!, a.lines, a.bot.id) },
      { label: "Open in Launchpad", hint: "The bot's chat and trace", onSelect: () => open(agentPath({ session_id: a.session }), ws) },
      { separator: true },
      { label: "Hide", hint: "Only on this page", onSelect: () => { asked.delete(keyOf(a.lines)); refreshPage(); } },
    ]));
    card.append(actions);
    const choice = choiceRow(id);
    if (choice) card.append(choice);
    const err = errors.get(id);
    if (err) card.append(h("div", { class: "lp-error", text: err, attrs: { role: "alert" } }));
    w.root.append(card);
  }
  for (const el of document.querySelectorAll<HTMLElement>('[data-launchpad^="answer-"]')) if (!ids.has(el.dataset.launchpad ?? "")) el.closest("tr[data-launchpad-row]")?.remove() ?? el.remove();
  return live;
}

/** The line comment the bot suggested for these lines (a suggested review of its session with a comment on them). */
function lineSuggestion(ctx: ExtContext | null, a: Asked): { s: ExtSuggestion; line: ExtLine } | null {
  for (const s of ctx?.suggestions ?? []) {
    if (s.kind !== "review" || s.session_id !== a.session) continue;
    const line = s.lines.find((l) => l.path === a.lines.path && l.side === a.lines.side && l.line === a.lines.end);
    if (line) return { s, line };
  }
  return null;
}

const pendingKey = (l: { path: string; side: Side; end: number }) => `${l.path}|${l.side}|${l.end}`;

function turnIntoComment(page: FilesPage, a: Asked, source: { s: ExtSuggestion; line: ExtLine } | null, text: string, id: string) {
  const attribution = source?.s.attribution ?? `_Written by ${a.bot.name} via ${LINK}_`;
  const params: Params = { repo: page.repo, number: page.number, path: a.lines.path, side: a.lines.side, line: a.lines.end };
  fillLine(params, (box, mode) => fillPart(box, source?.s.id ?? null, source?.line.part ?? "body", text, attribution, mode), id);
}

/** Fills GitHub's line form of `params` now, or once the owner opened it (2 minutes). Asks first when it has text. */
function fillLine(params: Params, fill: (box: HTMLTextAreaElement, mode?: "replace" | "append") => Promise<void>, id: string) {
  errors.delete(id);
  const box = findAnchor("line-form", params)?.el;
  const run = (b: HTMLTextAreaElement) => attempt(id, (mode) => fill(b, mode));
  if (box instanceof HTMLTextAreaElement) { void run(box); return; }
  const key = pendingKey({ path: params.path!, side: params.side!, end: params.line! });
  pending.set(key, { params, until: Date.now() + 120_000, fill: (b) => run(b) });
  findLine(params)?.scrollIntoView({ block: "center" });
  refreshPage();
}

/** Pending fills whose form the owner opened since. */
function fillPending(page: FilesPage) {
  for (const [key, p] of pending) {
    if (Date.now() > p.until) { pending.delete(key); continue; }
    const box = findAnchor("line-form", { ...p.params, repo: page.repo, number: page.number })?.el;
    if (box instanceof HTMLTextAreaElement) {
      pending.delete(key);
      void p.fill(box);
    }
  }
}

// ---- D2 ghost comments, D4 outdated guard --------------------------------------------------------------------------

function renderGhosts(page: FilesPage, ctx: ExtContext | null, reviews: ExtSuggestion[]) {
  const ids = new Set<string>();
  const head = ctx?.page.head ?? null;
  for (const s of reviews) {
    const outdated = !!(s.head && head && s.head !== head);
    s.lines.forEach((l, i) => {
      const id = `ghost-${s.id}-${i}`;
      const row = findLine({ repo: page.repo, number: page.number, path: l.path, side: l.side as Side, line: l.line });
      if (!row) return;
      ids.add(id);
      const mine = partState(s.id, l.part);
      const waiting = pending.has(pendingKey({ path: l.path, side: l.side as Side, end: l.line }));
      const key = [s.id, l.part, l.state, mine, outdated, confirmed.has(id), waiting, errors.get(id), choices.has(id)].join("|");
      const w = underLine(row, id, key, "card");
      if (!w.fresh) return;
      ghost(w.root, page, ctx!, s, l, id, outdated, mine, waiting);
      watchShown(w.host, s.id, l.part);
    });
  }
  for (const el of document.querySelectorAll<HTMLElement>('[data-launchpad^="ghost-"]')) if (!ids.has(el.dataset.launchpad ?? "")) el.closest("tr[data-launchpad-row]")?.remove() ?? el.remove();
}

function ghost(root: ShadowRoot, page: FilesPage, ctx: ExtContext, s: ExtSuggestion, l: ExtLine, id: string, outdated: boolean, mine: "in" | "posted" | null, waiting: boolean) {
  const ws = ctx.folder?.workspace_id ?? ctx.workspace?.id ?? null;
  const done = mine === "posted" || l.state === "posted_by_hand" || l.state === "confirmed";
  const state = done ? "Added to your review" : mine === "in" || l.state === "inserted" ? "Inserted in GitHub's form" : null;
  const where = l.start_line ? `Lines ${l.start_line}–${l.line}` : `Line ${l.line}`;
  const head = h("div", { class: "lp-row" }, logo(), h("span", { class: "lp-title", text: s.bot ?? "An agent" }), h("span", { class: "lp-muted lp-grow", text: `suggested line comment · ${where}` }));
  if (outdated) head.append(h("span", { class: "lp-tag lp-need", text: `outdated · at ${short(s.head)}`, title: `Written for commit ${s.head}; the pull request is now at ${ctx.page.head}. The line may have moved or changed.` }));
  if (state) head.append(h("span", { class: "lp-tag lp-ok", text: state }));
  const box = h("div", { class: "lp-ghost" }, head, h("div", { class: "lp-answer", text: l.body }));
  const actions = h("div", { class: "lp-actions" });
  const params: Params = { repo: page.repo, number: page.number, path: l.path, side: l.side as Side, line: l.line };
  const add = () => {
    // D4: an outdated comment is never inserted on the first click.
    if (outdated && !confirmed.has(id)) { confirmed.add(id); refreshPage(); return; }
    fillLine(params, (b, mode) => fillPart(b, s.id, l.part, l.body, s.attribution, mode), id);
  };
  if (waiting) actions.append(h("span", { class: "lp-muted lp-grow", text: `Open GitHub's comment form on line ${l.line} (its +): Launchpad fills it then.` }));
  else if (!done) {
    const label = outdated && confirmed.has(id) ? `Insert anyway (written at ${short(s.head)})` : mine || l.state === "inserted" ? "Add again" : "Add to my review";
    actions.append(button(label, add, { primary: !mine && !outdated, small: true, title: "Fills GitHub's comment form of this line: you add it to your review with GitHub's button" }));
  }
  if (l.start_line && !done) actions.append(h("span", { class: "lp-muted", text: `Select lines ${l.start_line}–${l.line} with GitHub's + for a range` }));
  actions.append(more(root, "Suggested line comment", (): MenuEntry[] => [
    { label: "Dismiss", hint: "Hidden here and left out when Launchpad posts the review", onSelect: () => void request("suggestion.ledger", { action: "dismiss-comment", id: s.id, part: l.part }).then(refreshPage).catch((e) => { errors.set(id, message(e)); refreshPage(); }) },
    { label: "Review in Launchpad", hint: "The whole suggested review", onSelect: () => open(`/github/${page.repo}/${page.number}?tab=files&draft=${encodeURIComponent(s.id)}`, ws) },
    ...(s.session_id ? [{ label: "Open the agent", hint: "Its chat and trace", onSelect: () => open(agentPath({ session_id: s.session_id! }), ws) }] : []),
  ]));
  box.append(actions);
  const choice = choiceRow(id);
  if (choice) box.append(choice);
  const err = errors.get(id);
  if (err) box.append(h("div", { class: "lp-error", text: err, attrs: { role: "alert" } }));
  root.append(box);
}

/** The banner over the diff when a suggested review exists: who reviewed at which head, its event, its line comments
 * (outdated ones counted), and Next comment. */
function renderReviewBanner(page: FilesPage, ctx: ExtContext | null, reviews: ExtSuggestion[]) {
  const s = reviews.at(-1);
  const first = findAnchors("diff-files", { repo: page.repo, number: page.number })?.els[0];
  if (!s || !ctx || !first) return unmount("review-banner");
  const anchor = first.closest("copilot-diff-entry") ?? first;
  const head = ctx.page.head ?? null;
  const outdated = !!(s.head && head && s.head !== head);
  const added = s.lines.filter((l) => partState(s.id, l.part) === "posted" || l.state === "posted_by_hand" || l.state === "confirmed").length;
  const key = [s.id, s.status, s.lines.length, added, outdated, head].join("|");
  const w = mount("review-banner", key, anchor, "beforebegin", "banner");
  if (!w.fresh) return;
  const ws = ctx.folder?.workspace_id ?? ctx.workspace?.id ?? null;
  const event = s.event === "APPROVE" ? "Approve" : s.event === "REQUEST_CHANGES" ? "Request changes" : "Comment";
  const facts = [
    `reviewed at ${short(s.head)}${outdated ? ` (the pull request is now at ${short(head)})` : " (current head)"}`,
    event,
    `${s.lines.length} line comment${s.lines.length === 1 ? "" : "s"}${added ? `, ${added} added` : ""}${outdated ? ", outdated" : ""}`,
    s.text.trim() ? "summary ready" : null,
  ].filter(Boolean).join(" · ");
  const row = h("div", { class: "lp-row" }, logo(), h("span", { class: "lp-title", text: s.bot ?? "An agent" }), h("span", { class: "lp-grow", text: facts, title: facts }));
  const next = () => {
    const ghosts = Array.from(document.querySelectorAll<HTMLElement>(`[data-launchpad^="ghost-${CSS.escape(s.id)}-"]`));
    const target = ghosts.find((g) => g.getBoundingClientRect().top > 80) ?? ghosts[0];
    target?.scrollIntoView({ block: "center" });
  };
  if (s.lines.length) row.append(button("Next comment", next, { small: true, title: "Scrolls to the next suggested line comment" }));
  row.append(more(w.root, "Suggested review", (): MenuEntry[] => [
    { label: "Review in Launchpad", hint: "Edit it, or post it as it is from there", onSelect: () => open(`/github/${page.repo}/${page.number}?tab=files&draft=${encodeURIComponent(s.id)}`, ws) },
    ...(s.session_id ? [{ label: "Open the agent", hint: "Its chat and trace", onSelect: () => open(agentPath({ session_id: s.session_id! }), ws) }] : []),
    { note: "Its summary goes into GitHub's Review changes dialog: open it, then Insert summary." },
  ]));
  w.root.append(h("div", { class: "lp-banner" }, row));
}

// ---- D3 summary into Review changes --------------------------------------------------------------------------------

function renderSummary(page: FilesPage, ctx: ExtContext | null, reviews: ExtSuggestion[]) {
  const s = reviews.filter((x) => x.text.trim()).at(-1);
  const found = s ? findAnchor("review-dialog", { repo: page.repo, number: page.number }) : null;
  if (!s || !ctx || !found || !(found.el instanceof HTMLTextAreaElement)) return unmount("review-summary");
  const box = found.el;
  const mine = partState(s.id, "body");
  const w = mount("review-summary", [s.id, s.status, mine, errors.get("review-summary"), choices.has("review-summary")].join("|"), box, "beforebegin", "banner");
  if (!w.fresh) return;
  const event = s.event === "APPROVE" ? "Approve" : s.event === "REQUEST_CHANGES" ? "Request changes" : "Comment";
  const title = mine === "posted" ? "Recorded: submitted by you" : mine === "in" || s.status === "inserted" ? "Summary inserted: edit it, choose the event, submit with GitHub's button" : `${s.bot ?? "An agent"} wrote a summary · suggests ${event}`;
  const row = h("div", { class: "lp-row" }, logo(), h("span", { class: "lp-grow", text: title }));
  if (mine !== "posted") row.append(button(mine || s.status === "inserted" ? "Insert again" : "Insert summary", () => {
    void attempt("review-summary", (mode) => fillPart(box, s.id, "body", s.text, s.attribution, mode));
  }, { primary: !mine && s.status !== "inserted", small: true, title: "Fills GitHub's summary field; you choose the event and submit with GitHub's button" }));
  const preview = h("div", { class: "lp-preview", text: s.text });
  const err = errors.get("review-summary");
  w.root.append(h("div", { class: "lp-banner" }, row, h("div", { class: "lp-muted", text: `Suggested event: ${event}. Choose it yourself below; Launchpad selects nothing.`, attrs: { style: "margin-top:4px" } }), preview, choiceRow("review-summary"), err ? h("div", { class: "lp-error", text: err }) : null));
  watchShown(w.host, s.id, "body");
}
