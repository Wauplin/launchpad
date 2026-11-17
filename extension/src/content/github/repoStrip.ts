// R2 Launchpad strip under a repository's tabs (its Code page): agents working, suggestions to post, Local PRs to
// publish, branches not on GitHub yet, shown only when one of them exists; one button, Launch, with the repository as
// context. Suggestions to post and Local PRs to publish unfold in place (each suggestion with its item and text, each
// Local PR with Publish… and its details), so the owner sees what waits without leaving GitHub. Nothing is added to
// GitHub's own tab bar: an extra element in it makes GitHub fold every tab into its More menu.

import type { ExtContext } from "@/gen/ExtContext";
import type { ExtSuggestion } from "@/gen/ExtSuggestion";
import { findAnchor } from "./anchors";
import { agentPath, elapsed, open, running, suggestionPath } from "./item";
import { prRow } from "./lists";
import type { Page } from "./page";
import { targetPopover } from "./target";
import { button, caret, h, logo, more, mount, unmount } from "./widgets";

const STRIP = "repo-strip";
/** Suggestions waiting for the owner (posted and discarded ones are done). */
const WAITING = ["draft", "inserted", "uncertain"];
/** The parts of the strip unfolded in this tab: `suggestions`, `prs`. */
const unfolded = new Set<string>();

function link(text: string, title: string, go: () => void) {
  return h("button", { class: "lp-link", text, title, attrs: { type: "button" }, on: { click: (e) => { e.preventDefault(); e.stopPropagation(); go(); } } });
}

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** A ▾ toggle of the strip that shows or hides `part` (kept unfolded across re-renders). */
function toggle(name: string, text: string, title: string, part: HTMLElement) {
  const b = h("button", { class: "lp-toggle", title, attrs: { type: "button", "aria-expanded": "false" } }, h("span", { text }), caret());
  const show = (on: boolean) => {
    b.setAttribute("aria-expanded", String(on));
    part.hidden = !on;
    if (on) unfolded.add(name); else unfolded.delete(name);
  };
  b.addEventListener("click", (e) => { e.preventDefault(); e.stopPropagation(); show(b.getAttribute("aria-expanded") !== "true"); });
  show(unfolded.has(name));
  return b;
}

/** The item of a suggestion on GitHub (GitHub sends `/issues/<n>` of a pull request to its page). */
const itemUrl = (repo: string, s: ExtSuggestion) => `https://github.com/${repo}/issues/${s.number}`;

function suggestionRow(root: ShadowRoot, repo: string, s: ExtSuggestion, ws: string) {
  const review = s.kind === "review";
  const what = review ? `Review${s.comments ? ` · ${s.comments} line comment${s.comments === 1 ? "" : "s"}` : ""}${s.event && s.event !== "COMMENT" ? ` · ${s.event.toLowerCase().replace("_", " ")}` : ""}` : s.reply_to ? "Reply" : "Comment";
  const text = s.body.replace(/\s+/g, " ").trim();
  // Comments and reviews can be posted from the item's timeline.
  const go = review
    ? button("Open item", () => { location.assign(itemUrl(repo, s)); }, { small: true, title: "Post the review in the item’s timeline" })
    : button("Open item", () => { location.assign(itemUrl(repo, s)); }, { small: true, title: "The comment waits in the item's timeline, with Post this comment" });
  const menu = more(root, "Suggestion", () => [
    review ? { label: "Open the item on GitHub", onSelect: () => { location.assign(itemUrl(repo, s)); } } : { label: "Open in Launchpad", hint: "Its card in Today", onSelect: () => open(suggestionPath(s.id), ws) },
    ...(s.session_id ? [{ label: "Open its agent", onSelect: () => open(agentPath({ session_id: s.session_id! }), ws) }] : []),
  ]);
  return h("div", { class: "lp-detail-row" },
    h("div", { class: "lp-main" },
      h("div", { class: "lp-row" }, h("a", { class: "lp-link", text: `#${s.number}`, attrs: { href: itemUrl(repo, s) } }), h("span", { class: "lp-muted lp-clip", text: `${what} · ${s.author} · ${elapsed(s.created_at)}` })),
      h("div", { class: "lp-clip", text: text || "(empty)", title: text.slice(0, 600) })),
    go, menu);
}

export function renderRepoStrip(page: Page, ctx: ExtContext | null, refresh: () => void) {
  const mine = ctx && page.repo && ctx.page.repo?.toLowerCase() === page.repo.toLowerCase() && ctx.folder && page.number === null ? ctx : null;
  if (!mine || page.kind !== "repo") return unmount(STRIP);
  const f = mine.folder!;
  const ws = f.workspace_id;
  const agents = mine.agents.filter(running);
  const suggestions = mine.suggestions.filter((s) => WAITING.includes(s.status));
  const prs = mine.local_prs.filter((p) => p.state === "open");
  const behind = mine.branches.filter((b) => b.not_on_github);
  if (!agents.length && !suggestions.length && !prs.length && !behind.length) return unmount(STRIP);
  const anchor = findAnchor("repo-content", { repo: page.repo!, number: null });
  if (!anchor) return unmount(STRIP);
  const key = JSON.stringify([page.repo, agents.map((a) => a.session_id), suggestions.map((s) => [s.id, s.status, s.body.length]), prs.map((p) => [p.id, p.title, p.notes, p.pushed_head]), behind.map((b) => `${b.branch}:${b.head}`)]);
  const w = mount(STRIP, key, anchor.el, anchor.place, "strip");
  if (!w.fresh) return;
  const items: HTMLElement[] = [];
  const parts: HTMLElement[] = [];
  if (agents.length) items.push(h("span", { class: "lp-row" }, h("span", { class: "lp-dot lp-run" }), link(plural(agents.length, "agent working", "agents working"), agents.map((a) => a.title).join("\n"), () => open(agents.length === 1 ? agentPath(agents[0]) : "/agents", ws))));
  if (suggestions.length) {
    const part = h("div", { class: "lp-details" }, ...suggestions.slice(0, 20).map((s) => suggestionRow(w.root, page.repo!, s, ws)));
    if (suggestions.length > 20) part.append(h("div", { class: "lp-muted", text: `and ${suggestions.length - 20} more in Launchpad's Today` }));
    items.push(h("span", { class: "lp-row" }, h("span", { class: "lp-dot lp-need" }), toggle("suggestions", plural(suggestions.length, "suggestion to post", "suggestions to post"), "Agent-written comments and reviews waiting for you: show them", part)));
    parts.push(part);
  }
  if (prs.length) {
    const part = h("div", { class: "lp-details" }, ...prs.map((p) => prRow(w.root, p, ws, refresh)));
    items.push(h("span", { class: "lp-row" }, h("span", { class: "lp-dot lp-need" }), toggle("prs", `${plural(prs.length, "Local PR", "Local PRs")} to publish`, "Local PRs not on GitHub yet (only you see them): show them", part)));
    parts.push(part);
  }
  for (const b of behind.slice(0, 2)) items.push(h("span", { class: "lp-row", title: b.error ?? "Home has commits GitHub does not have yet" }, h("span", { class: b.error ? "lp-dot lp-bad" : "lp-dot lp-need" }), h("a", { class: "lp-link", text: `${b.branch} not on GitHub yet`, attrs: { href: b.url } })));
  const launch: HTMLButtonElement = button("Launch", () => targetPopover(w.root, launch, { targets: [{ repo: page.repo!, number: null, kind: "repo", title: null }], kind: "ask", bot: mine.defaults.bot?.id ?? null, presets: mine.presets, where: `${f.workspace} · ${f.name}`, done: refresh }), { small: true, title: "Ask a bot of the workspace, with this repository as context" });
  w.root.append(h("div", { class: "lp-banner lp-strip" }, logo(), ...items, h("span", { class: "lp-grow" }), launch, ...parts));
}
