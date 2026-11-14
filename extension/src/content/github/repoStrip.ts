// R2 Launchpad strip under a repository's tabs (its Code page): agents working, suggestions to post, Local PRs to
// publish, branches not on GitHub yet, shown only when one of them exists; each opens its place; one button, Launch,
// with the repository as context. L1's "+2 local" after the Pull requests tab: Local PRs not on GitHub yet.

import type { ExtContext } from "@/gen/ExtContext";
import { findAnchor } from "./anchors";
import { agentPath, open, running, suggestionPath } from "./item";
import type { Page } from "./page";
import { targetPopover } from "./target";
import { button, h, logo, mount, unmount } from "./widgets";

const STRIP = "repo-strip";
const LOCAL = "pr-tab-local";

function link(text: string, title: string, go: () => void) {
  return h("button", { class: "lp-link", text, title, attrs: { type: "button" }, on: { click: (e) => { e.preventDefault(); e.stopPropagation(); go(); } } });
}

export function renderRepoStrip(page: Page, ctx: ExtContext | null, refresh: () => void) {
  const mine = ctx && page.repo && ctx.page.repo?.toLowerCase() === page.repo.toLowerCase() && ctx.folder && page.number === null ? ctx : null;
  renderLocalCount(page, mine);
  if (!mine || page.kind !== "repo") return unmount(STRIP);
  const f = mine.folder!;
  const ws = f.workspace_id;
  const agents = mine.agents.filter(running);
  const suggestions = mine.suggestions;
  const prs = mine.local_prs.filter((p) => p.state === "open");
  const behind = mine.branches.filter((b) => b.not_on_github);
  if (!agents.length && !suggestions.length && !prs.length && !behind.length) return unmount(STRIP);
  const anchor = findAnchor("repo-content", { repo: page.repo!, number: null });
  if (!anchor) return unmount(STRIP);
  const key = JSON.stringify([page.repo, agents.map((a) => a.session_id), suggestions.map((s) => s.id), prs.map((p) => p.id), behind.map((b) => `${b.branch}:${b.head}`)]);
  const w = mount(STRIP, key, anchor.el, anchor.place, "strip");
  if (!w.fresh) return;
  const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;
  const items: HTMLElement[] = [];
  if (agents.length) items.push(h("span", { class: "lp-row" }, h("span", { class: "lp-dot lp-run" }), link(plural(agents.length, "agent working", "agents working"), agents.map((a) => a.title).join("\n"), () => open(agents.length === 1 ? agentPath(agents[0]) : "/agents", ws))));
  if (suggestions.length) {
    const s = suggestions[0];
    items.push(h("span", { class: "lp-row" }, h("span", { class: "lp-dot lp-need" }), link(plural(suggestions.length, "suggestion to post", "suggestions to post"), "Agent-written comments and reviews waiting for you", () => open(suggestions.length === 1 ? suggestionPath(s.id) : "/?filter=github", ws))));
  }
  if (prs.length) items.push(h("span", { class: "lp-row" }, h("span", { class: "lp-dot lp-need" }), link(`${plural(prs.length, "Local PR", "Local PRs")} to publish`, prs.map((p) => p.title).join("\n"), () => { location.assign(`/${page.repo}/pulls`); })));
  for (const b of behind.slice(0, 2)) items.push(h("span", { class: "lp-row", title: b.error ?? "Home has commits GitHub does not have yet" }, h("span", { class: b.error ? "lp-dot lp-bad" : "lp-dot lp-need" }), h("a", { class: "lp-link", text: `${b.branch} not on GitHub yet`, attrs: { href: b.url } })));
  const launch: HTMLButtonElement = button("Launch", () => targetPopover(w.root, launch, { targets: [{ repo: page.repo!, number: null, kind: "repo", title: null }], kind: "ask", bot: mine.defaults.bot?.id ?? null, presets: mine.presets, where: `${f.workspace} · ${f.name}`, done: refresh }), { small: true, title: "Ask a bot of the workspace, with this repository as context" });
  w.root.append(h("div", { class: "lp-banner lp-strip" }, logo(), ...items, h("span", { class: "lp-grow" }), launch));
}

function renderLocalCount(page: Page, ctx: ExtContext | null) {
  const n = ctx?.local_prs.filter((p) => p.state === "open").length ?? 0;
  if (!ctx || !n || !page.repo) return unmount(LOCAL);
  const anchor = findAnchor("pr-tab", { repo: page.repo, number: null });
  if (!anchor) return unmount(LOCAL);
  const w = mount(LOCAL, `${page.repo}:${n}`, anchor.el, anchor.place);
  if (!w.fresh) return;
  w.root.append(h("a", { class: "lp-count", text: `+${n} local`, title: `${n} Local PR${n === 1 ? "" : "s"} not on GitHub yet (only you see them): listed on the Pull requests page`, attrs: { href: `/${page.repo}/pulls` } }));
}
