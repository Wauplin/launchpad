// A repository's Pull requests and Issues lists (M2): L1 the Local PRs of the repository not on GitHub yet, above
// GitHub's list, each with Publish… (the same options and confirmation as Launchpad: remote branch, GitHub draft, from
// my fork; it publishes the commit the dialog showed); L2 one chip per row; L3 Review with agent (Ask on issues) on a
// row's hover, with the bot and preset last used on the repository; L4 rows selected with GitHub's own checkboxes
// (read, never changed) become the context of one agent.

import type { ExtContext } from "@/gen/ExtContext";
import type { ExtLocalPr } from "@/gen/ExtLocalPr";
import { request } from "../../shared/messages";
import { findAnchor, findAnchors, rowChecked, rowItem, rowSlot } from "./anchors";
import { open } from "./item";
import type { Page } from "./page";
import { chipButton, chipOf, hoverWith, RowStates } from "./rows";
import { targetPopover, type Target } from "./target";
import { button, h, logo, more, mount, popover, unmount, type MenuEntry } from "./widgets";

const SECTION = "local-prs";
const BAR = "selection-bar";
let states: RowStates | null = null;
/** Row hosts of the last render, to remove the ones whose row left. */
let shown = new Set<string>();

export const isList = (p: Page) => !!p.repo && (p.kind === "pulls" || p.kind === "issues");

/** Home's records changed: rows are asked again. */
export function listsStale() {
  states?.stale();
}

export function clearLists() {
  unmount(SECTION);
  unmount(BAR);
  for (const id of shown) unmount(id);
  shown = new Set();
  states?.clear();
}

export function renderLists(page: Page, ctx: ExtContext | null, rerender: () => void, refresh: () => void) {
  if (!isList(page)) return clearLists();
  const mine = ctx && ctx.page.repo?.toLowerCase() === page.repo!.toLowerCase() ? ctx : null;
  if (page.kind === "pulls") renderSection(page, mine, refresh); else unmount(SECTION);
  states ??= new RowStates(rerender);
  renderRows(page, mine, states);
}

// L1: the Local PRs section.

function renderSection(page: Page, ctx: ExtContext | null, refresh: () => void) {
  const prs = ctx?.folder ? ctx.local_prs.filter((p) => p.state === "open") : [];
  if (!ctx || !prs.length) return unmount(SECTION);
  const anchor = findAnchor("list-view", { repo: page.repo!, number: null });
  if (!anchor) return unmount(SECTION);
  const ws = ctx.folder!.workspace_id;
  const w = mount(SECTION, JSON.stringify([page.repo, prs]), anchor.el, anchor.place, "box");
  if (!w.fresh) return;
  const section = h("div", { class: "lp-section" },
    h("div", { class: "lp-section-head" }, logo(), h("span", { class: "lp-title", text: "Local PRs · not on GitHub yet" }), h("span", { class: "lp-muted", text: String(prs.length) }), h("span", { class: "lp-grow" }), h("span", { class: "lp-muted", text: "Only you see this" })));
  for (const p of prs) section.append(prRow(w.root, p, ws, refresh));
  w.root.append(section);
}

function prRow(root: ShadowRoot, p: ExtLocalPr, ws: string, refresh: () => void) {
  const meta = [`${p.branch} → ${p.base_branch}`, p.author ? `by ${p.author}` : null, p.notes ? `${p.notes} note${p.notes === 1 ? "" : "s"} waiting` : null, p.set ? `change set of ${p.set + 1}` : null, p.pushed_head ? "publication did not finish" : null].filter(Boolean).join(" · ");
  const publish: HTMLButtonElement = button("Publish…", () => publishPopover(root, publish, p, refresh), { small: true, title: "Push the reviewed commit and open the pull request on GitHub (you confirm first)" });
  const entries = (): MenuEntry[] => [
    { label: "Review in Launchpad", hint: "Its changes, commits and notes", onSelect: () => open(`/predrafts/${encodeURIComponent(p.id)}`, ws) },
    { label: "Merge on Home…", hint: "From its page in Launchpad", onSelect: () => open(`/predrafts/${encodeURIComponent(p.id)}`, ws) },
    ...(p.set ? [{ label: "Publish all…", hint: `The ${p.set + 1} Local PRs of its change set, from Launchpad`, onSelect: () => open(`/predrafts/${encodeURIComponent(p.id)}`, ws) }] : []),
    ...(p.session_id ? [{ label: "Open its agent", onSelect: () => open(`/agents?session=${encodeURIComponent(p.session_id!)}`, ws) }] : []),
    { separator: true },
    { label: "Discard…", hint: "From its page in Launchpad", onSelect: () => open(`/predrafts/${encodeURIComponent(p.id)}`, ws) },
  ];
  return h("div", { class: "lp-section-row" },
    h("div", { class: "lp-grow" }, h("div", { class: "lp-grow", text: p.title, title: p.title, attrs: { style: "font-weight:600" } }), h("div", { class: "lp-muted lp-grow", text: meta, title: meta })),
    publish, more(root, "Local PR", entries));
}

/** The Publish dialog: options, then a confirmation that names the commit; only the confirm click publishes. */
function publishPopover(root: ShadowRoot, anchor: HTMLElement, p: ExtLocalPr, refresh: () => void) {
  const status = h("div", { class: "lp-muted", text: "Reading the Local PR…" });
  const remote = h("input", { attrs: { type: "text", "aria-label": "Remote branch name", placeholder: "Remote branch name" } });
  const draft = h("input", { attrs: { type: "checkbox" } });
  draft.checked = true;
  const fork = h("input", { attrs: { type: "checkbox" } });
  const error = h("div", { class: "lp-error", attrs: { role: "alert", hidden: "" } });
  const confirm = h("div", { class: "lp-confirm", attrs: { hidden: "" } });
  const next = h("button", { class: "lp-btn lp-primary", text: "Publish…", attrs: { type: "button", disabled: "" } });
  const back = h("button", { class: "lp-btn lp-small", text: "Back", attrs: { type: "button", hidden: "" } });
  const options = h("div", { attrs: { style: "display:grid;gap:8px" } }, remote, h("label", {}, draft, "As a GitHub draft"), h("label", { title: "Push to your fork of origin and open the PR from it, when you cannot push to origin" }, fork, "From my fork"));
  let head: string | null = null;
  let confirming = false;
  let busy = false;
  const close = popover(root, anchor, `Publish ${p.title}`, (close) => [
    status, options, confirm, error,
    h("div", { class: "lp-row" }, h("span", { class: "lp-grow" }), back, button("Cancel", close, { small: true }), next),
  ]);
  const fail = (e: unknown) => { error.textContent = e instanceof Error ? e.message : String(e); error.hidden = false; };
  const setConfirming = (on: boolean) => {
    confirming = on;
    options.hidden = on;
    confirm.hidden = !on;
    back.hidden = !on;
    next.textContent = on ? "Push and create PR" : "Publish…";
    if (on) confirm.textContent = `Home pushes the reviewed commit ${head!.slice(0, 8)} as ${remote.value.trim()} to ${fork.checked ? "your fork" : "origin"} and creates ${draft.checked ? "a GitHub draft" : "a"} PR on GitHub.`;
  };
  back.addEventListener("click", (e) => { e.preventDefault(); setConfirming(false); });
  next.addEventListener("click", (e) => {
    e.preventDefault();
    // Only a real click of the owner publishes (never a click the page synthesized).
    if (!e.isTrusted || busy || !head || !remote.value.trim()) return;
    if (!confirming) return setConfirming(true);
    busy = true;
    next.disabled = true;
    next.textContent = "Publishing…";
    error.hidden = true;
    request("localpr.publish", { id: p.id, remote_branch: remote.value.trim(), draft: draft.checked, head, fork: fork.checked })
      .then(() => { close(); refresh(); })
      .catch((err) => { fail(err); busy = false; next.disabled = false; next.textContent = "Push and create PR"; });
  });
  void request("localpr.detail", { id: p.id }).then((d) => {
    const running = ["working", "starting", "background"].includes(d.status ?? "");
    remote.value = d.predraft.remote_branch ?? d.default_remote_branch ?? d.predraft.default_remote_branch;
    fork.checked = !!d.predraft.push_repo;
    fork.disabled = !!d.predraft.pushed_head;
    head = d.head ?? null;
    const notes = [
      head ? `Reviewed commit ${head.slice(0, 8)} · ${p.branch} → ${p.base_branch}` : "No commits to publish yet.",
      d.base_ahead > 0 ? `Base moved: ${p.base_branch} is ${d.base_ahead} commits ahead. Review against it in Launchpad first.` : null,
      d.change_set?.length ? `Part of a change set of ${d.change_set.length + 1}: Publish all in Launchpad publishes them together.` : null,
      running ? "An agent works on it: publish when it is done." : null,
    ].filter(Boolean).join("\n");
    status.textContent = notes;
    status.style.whiteSpace = "pre-line";
    next.disabled = !head || running;
  }).catch(fail);
}

// L2–L4: the rows.

function renderRows(page: Page, ctx: ExtContext | null, st: RowStates) {
  const found = findAnchors("list-items", { repo: page.repo!, number: null });
  const rows = (found?.els ?? []).map((el) => ({ el, item: rowItem(el) })).filter((r) => r.item && r.item.repo.toLowerCase() === page.repo!.toLowerCase());
  const data = rows.length && ctx?.folder ? st.get(page.url, rows.map((r) => String(r.item!.number))) : null;
  const ws = ctx?.folder?.workspace_id ?? ctx?.workspace?.id ?? null;
  const next = new Set<string>();
  for (const { el, item } of rows) {
    const id = `row-${item!.number}`;
    next.add(id);
    const state = data?.rows.find((r) => r.item === String(item!.number)) ?? null;
    const chip = state ? chipOf(state, ws, false) : null;
    const target: Target = { repo: item!.repo, number: item!.number, kind: item!.kind, title: item!.title };
    const slot = rowSlot(el);
    const w = mount(id, JSON.stringify([chip?.text, chip?.tone, !!ctx, ctx?.defaults.review, ctx?.defaults.bot?.id]), slot.el, slot.place);
    hoverWith(el, w);
    if (!w.fresh) continue;
    if (chip) w.root.append(chipButton(chip, () => undefined));
    if (ctx) {
      const review = item!.kind === "pr";
      const choice = ctx.defaults.review;
      const remembered = review && choice?.bot ? " (as last time on this repository)" : "";
      const act: HTMLButtonElement = button(review ? "Review with agent" : "Ask", () => targetPopover(w.root, act, { targets: [target], kind: review ? "review" : "ask", bot: ctx.defaults.bot?.id ?? null, presets: ctx.presets, remember: review ? { repo: item!.repo, choice } : undefined, where: ctx.folder ? `${ctx.folder.workspace} · ${ctx.folder.name}` : `${ctx.workspace?.name ?? "Workspace"} · no clone`, done: () => st.stale() }), { small: true, title: `${review ? "A suggested review for you to post" : "Ask a bot about this issue"}${remembered}` });
      w.root.append(h("span", { class: "lp-hover" }, act));
    }
  }
  for (const id of shown) if (!next.has(id)) unmount(id);
  shown = next;
  renderSelection(ctx, rows.filter((r) => rowChecked(r.el)).map((r) => ({ repo: r.item!.repo, number: r.item!.number, kind: r.item!.kind, title: r.item!.title })));
}

/** L4: two or more rows checked in GitHub's list → one agent with all of them as context. */
function renderSelection(ctx: ExtContext | null, picked: Target[]) {
  if (!ctx || picked.length < 2) return unmount(BAR);
  const w = mount(BAR, picked.map((t) => t.number).join(","), document.body, "beforeend");
  if (!w.fresh) return;
  const go: HTMLButtonElement = button("Launch one agent with them", () => targetPopover(w.root, go, { targets: picked, kind: "many", bot: ctx.defaults.bot?.id ?? null, presets: ctx.presets, where: ctx.folder ? `${ctx.folder.workspace} · ${ctx.folder.name}` : `${ctx.workspace?.name ?? "Workspace"} · no clone` }), { primary: true, small: true });
  w.root.append(h("div", { class: "lp-bar" }, logo(), h("span", { text: `${picked.length} selected` }), go));
}
