// I1 Launchpad box, the first block of a pull request's or issue's sidebar: the agents on this item, what waits for you
// (a suggestion, a Local PR), the branch state, and ONE main action (Review with agent on a pull request, Draft a reply
// on an issue, Continue when an agent works on it), ⋯ for the rest. Its space is reserved as soon as the page is known,
// so GitHub's sidebar does not jump when Home answers.

import type { ExtAgent } from "@/gen/ExtAgent";
import type { ExtContext } from "@/gen/ExtContext";
import { findAnchor } from "./anchors";
import { askPopover } from "./askPopover";
import { insertSuggestion } from "./insert";
import { agentPath, dotClass, elapsed, open, statusText, stop, who, type ItemPage } from "./item";
import { stepOfSession } from "./live";
import { button, h, logo, more, mount, unmount, type MenuEntry } from "./widgets";

const ID = "sidebar-box";

function agentRow(a: ExtAgent) {
  const step = a.status === "working" ? stepOfSession(a.session_id) : null;
  return h("div", { class: "lp-row", title: a.title },
    h("span", { class: dotClass(a) }),
    h("span", { class: "lp-grow", text: `${who(a)} · ${step ?? statusText(a)}` }),
    h("span", { class: "lp-muted", text: elapsed(a.since) }));
}

/** Renders the box (idempotent). `ctx` null: the page is known but Home did not answer yet (reserved space). */
export function renderSidebarBox(page: ItemPage, ctx: ExtContext | null, refresh: () => void) {
  const anchor = findAnchor("sidebar", { repo: page.repo, number: page.number });
  if (!anchor) return unmount(ID);
  const ws = ctx?.folder?.workspace_id ?? ctx?.workspace?.id ?? null;
  const agents = ctx?.agents.filter((a) => a.number === page.number || a.number === null) ?? [];
  const waiting = ctx?.suggestions.filter((s) => s.number === page.number) ?? [];
  const branches = ctx?.branches ?? [];
  const steps = agents.map((a) => `${a.session_id}:${a.status}:${a.since}:${stepOfSession(a.session_id) ?? ""}`).join(",");
  const key = ctx ? JSON.stringify([page.url, ctx.folder?.id, ctx.workspace?.id, steps, waiting.map((s) => `${s.id}:${s.status}`), branches.map((b) => `${b.branch}:${b.not_on_github}:${b.error}`), ctx.local_prs.map((p) => `${p.id}:${p.state}`)]) : `reserve:${page.url}`;
  const w = mount(ID, key, anchor.el, anchor.place, "box");
  if (!w.fresh) return;
  if (!ctx) {
    w.root.append(h("div", { class: "lp-box lp-reserve", attrs: { "aria-busy": "true" } }, h("div", { class: "lp-row" }, logo(), h("span", { class: "lp-title", text: "Launchpad" }), h("span", { class: "lp-muted", text: "…" }))));
    return;
  }
  const where = ctx.folder ? `${ctx.folder.workspace} · ${ctx.folder.name}` : `${ctx.workspace?.name ?? "Launchpad"} · not a folder`;
  const box = h("div", { class: "lp-box" }, h("div", { class: "lp-row" }, logo(), h("span", { class: "lp-title", text: "Launchpad" }), h("span", { class: "lp-muted lp-grow", text: where })));

  if (agents.length) {
    box.append(h("div", { class: "lp-label", text: `Agents on this ${page.kind === "issue" ? "issue" : "pull request"}` }), ...agents.slice(0, 3).map(agentRow));
    if (agents.length > 3) box.append(h("div", { class: "lp-muted", text: `and ${agents.length - 3} more` }));
  }
  const reply = waiting.find((s) => s.kind === "comment");
  const review = waiting.find((s) => s.kind === "review");
  const prs = ctx.local_prs.filter((p) => p.state === "open");
  if (waiting.length || prs.length) {
    box.append(h("div", { class: "lp-label", text: "Waiting for you" }));
    for (const s of waiting) {
      const label = s.kind === "review" ? `Review ready${s.outdated ? ` · ${s.outdated} outdated` : ""}` : s.reply_to ? `Reply to ${s.reply_to.author ?? "a comment"}` : "Comment ready";
      box.append(h("div", { class: "lp-row" }, h("span", { class: "lp-dot lp-need" }), h("span", { class: "lp-grow", text: `${label}${s.bot ? ` · ${s.bot}` : ""}${s.status === "inserted" ? " · inserted" : ""}` })));
    }
    for (const p of prs) box.append(h("div", { class: "lp-row" }, h("span", { class: "lp-dot lp-need" }), h("span", { class: "lp-grow", text: `Local PR · ${p.title}` })));
  }
  const behind = branches.filter((b) => b.not_on_github || b.error);
  if (behind.length) {
    box.append(h("div", { class: "lp-label", text: "Branch" }));
    for (const b of behind) box.append(h("div", { class: "lp-row", title: b.error ?? undefined }, h("span", { class: b.error ? "lp-dot lp-bad" : "lp-dot lp-need" }), h("span", { class: "lp-grow", text: `${b.branch} · ${b.error ? `push refused: ${b.error}` : "not on GitHub yet"}` })));
  }

  // One main action: Insert a waiting reply, else Continue with an agent, else Review with agent / Draft a reply.
  const actions = h("div", { class: "lp-actions" });
  const latest = agents[0] ?? null;
  let main: HTMLButtonElement;
  if (reply && reply.status === "draft") {
    main = button("Insert reply", () => void insertSuggestion(page, reply).catch((e) => console.warn("Launchpad: insert", e)), { full: true, title: "Puts the reply into GitHub's box: you edit it and post it with GitHub's button" });
  } else if (latest) {
    main = button("Continue", () => {
      if (latest.bot_id) askPopover(w.root, main, page, ctx, null, "ask", refresh, latest.bot_id);
      else open(agentPath(latest), ws);
    }, { full: true, title: latest.bot ? `Message ${latest.bot} about this item` : "Open the agent in Launchpad" });
  } else {
    const kind = page.kind === "issue" ? "reply" : "review";
    main = button(kind === "review" ? "Review with agent" : "Draft a reply", () => askPopover(w.root, main, page, ctx, null, kind, refresh), { full: true, title: ctx.defaults.bot ? `With ${ctx.defaults.bot.name}` : undefined });
  }
  actions.append(main, more(w.root, "Launchpad", () => entries(page, ctx, agents, ws, refresh, w.root, main, review?.id ?? null)));
  box.append(actions);
  w.root.append(box);
}

function entries(page: ItemPage, ctx: ExtContext, agents: ExtAgent[], ws: string | null, refresh: () => void, root: ShadowRoot, anchor: HTMLElement, review: string | null): MenuEntry[] {
  const out: MenuEntry[] = [];
  out.push({ label: "Ask a bot…", hint: "A bot of the workspace, with this item as context", onSelect: () => askPopover(root, anchor, page, ctx, null, "ask", refresh) });
  if (page.kind !== "issue") out.push({ label: "Review with agent", hint: "A suggested review for you to post", onSelect: () => askPopover(root, anchor, page, ctx, null, "review", refresh) });
  else out.push({ label: "Draft a reply", hint: "A suggested comment for you to post", onSelect: () => askPopover(root, anchor, page, ctx, null, "reply", refresh) });
  if (review) out.push({ label: "Open the suggested review", hint: "In Launchpad, with its line comments", onSelect: () => open(`/github/${page.repo}/${page.number}?tab=files&draft=${encodeURIComponent(review)}`, ws) });
  for (const a of agents.slice(0, 5)) {
    out.push({ separator: true });
    out.push({ label: `Open ${who(a)} in Launchpad`, hint: a.title, onSelect: () => open(agentPath(a), ws) });
    if (a.status === "working" || a.status === "starting") out.push({ label: `Stop ${who(a)}`, hint: "Stops its turn", onSelect: () => void stop(a).then(refresh).catch((e) => console.warn("Launchpad: stop", e)) });
  }
  for (const p of ctx.local_prs.slice(0, 3)) out.push({ label: `Open Local PR ${p.title}`, hint: p.state, onSelect: () => open(`/predrafts/${encodeURIComponent(p.id)}`, ws) });
  out.push({ separator: true });
  out.push({ label: "Open in Launchpad", hint: "This item in Launchpad", onSelect: () => open(`/github/${page.repo}/${page.number}`, ws) });
  return out;
}
