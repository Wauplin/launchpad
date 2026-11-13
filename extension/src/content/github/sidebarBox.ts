// I1 Launchpad box, the first block of a pull request's or issue's sidebar: the agents on this item, what waits for you
// (a suggestion, a Local PR), the branch state (I7: an agent branch ready to push to this pull request, "Not on GitHub
// yet" with the last error, whether maintainers can edit a fork's branch), and ONE main action, ⋯ for the rest. On a
// pull request: Insert reply, else Push to this PR, else Continue, else Review with agent. On an issue (I6): Insert
// reply, else Publish… of its Local PR, else Fix in a Local PR once an agent looked at it, else Investigate. Its space
// is reserved as soon as the page is known, so GitHub's sidebar does not jump when Home answers.

import type { ExtAgent } from "@/gen/ExtAgent";
import type { ExtBranch } from "@/gen/ExtBranch";
import type { ExtContext } from "@/gen/ExtContext";
import { request } from "../../shared/messages";
import { findAnchor } from "./anchors";
import { askPopover } from "./askPopover";
import { openAgentView } from "./agentView";
import { insertSuggestion, isPrefilled, showPrefilled } from "./insert";
import { agentPath, dotClass, elapsed, open, refOf, running, statusText, stop, who, type ItemPage } from "./item";
import { launchPopover } from "./launchPopover";
import { publishPopover } from "./lists";
import { stepOfSession } from "./live";
import { button, h, logo, more, mount, popover, unmount, type MenuEntry } from "./widgets";

const ID = "sidebar-box";

function agentRow(a: ExtAgent, ws: string | null) {
  const step = a.status === "working" ? stepOfSession(a.session_id) : null;
  const show = h("button", { class: "lp-link lp-grow", text: `${who(a)} · ${step ?? statusText(a)}`, title: `${a.title}: show its work here`, attrs: { type: "button", style: "text-align: left" },
    on: { click: (e) => { e.preventDefault(); openAgentView({ session_id: a.session_id, title: a.title, bot: a.bot, running: running(a) }, ws); } } });
  return h("div", { class: "lp-row" },
    h("span", { class: dotClass(a) }),
    show,
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
  const key = ctx ? JSON.stringify([page.url, ctx.folder?.id, ctx.workspace?.id, steps, waiting.map((s) => `${s.id}:${s.status}:${s.outdated}:${isPrefilled(s.id)}`), branches.map((b) => `${b.branch}:${b.head}:${b.not_on_github}:${b.error}:${b.push_ready?.ahead}`), ctx.local_prs.map((p) => `${p.id}:${p.state}`), ctx.page.maintainer_can_modify, ctx.page.head_repo]) : `reserve:${page.url}`;
  const w = mount(ID, key, anchor.el, anchor.place, "box");
  if (!w.fresh) return;
  if (!ctx) {
    w.root.append(h("div", { class: "lp-box lp-reserve", attrs: { "aria-busy": "true" } }, h("div", { class: "lp-row" }, logo(), h("span", { class: "lp-title", text: "Launchpad" }), h("span", { class: "lp-muted", text: "…" }))));
    return;
  }
  const where = ctx.folder ? `${ctx.folder.workspace} · ${ctx.folder.name}` : `${ctx.workspace?.name ?? "Launchpad"} · not a folder`;
  const box = h("div", { class: "lp-box" }, h("div", { class: "lp-row" }, logo(), h("span", { class: "lp-title", text: "Launchpad" }), h("span", { class: "lp-muted lp-grow", text: where })));

  if (agents.length) {
    box.append(h("div", { class: "lp-label", text: `Agents on this ${page.kind === "issue" ? "issue" : "pull request"}` }), ...agents.slice(0, 3).map((a) => agentRow(a, ws)));
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
  const branchRows = page.kind === "issue" ? [] : branchState(ctx, branches);
  if (branchRows.length) box.append(h("div", { class: "lp-label", text: "Branch" }), ...branchRows);
  const ready = page.kind === "issue" ? null : branches.find((b) => b.push_ready) ?? null;
  const pushable = !!ready && canPush(ctx);

  // One main action (see the top of this file).
  const actions = h("div", { class: "lp-actions" });
  const latest = agents[0] ?? null;
  const ref = refOf(page, ctx);
  const repoFolder = !!ctx.folder && ctx.folder.relation !== "fork";
  let main: HTMLButtonElement;
  if (reply && reply.status === "draft" && isPrefilled(reply.id)) {
    main = button("Edit the reply", () => showPrefilled(reply.id), { full: true, title: "The draft is in GitHub's comment box: edit it, then post it with GitHub's button" });
  } else if (reply && reply.status === "draft") {
    main = button("Insert reply", () => void insertSuggestion(page, reply).catch((e) => console.warn("Launchpad: insert", e)), { full: true, title: "Puts the reply into GitHub's box: you edit it and post it with GitHub's button" });
  } else if (page.kind === "issue" && prs[0]) {
    const p = prs[0];
    main = button("Publish…", () => publishPopover(w.root, main, p, refresh), { full: true, title: `${p.title}: push the reviewed commit and open the pull request on GitHub (you confirm first)` });
  } else if (page.kind === "issue" && repoFolder) {
    const kind = latest ? "fix" : "investigate";
    main = button(kind === "fix" ? "Fix in a Local PR" : "Investigate", () => launchPopover(w.root, main, ctx, kind, ref, refresh), { full: true, title: kind === "fix" ? "An agent fixes it in its own worktree and opens a Local PR linked to this issue" : "A read-only agent finds the root cause" });
  } else if (ready && pushable) {
    main = button("Push to this PR", () => pushDialog(w.root, main, page, ctx, ready, refresh), { full: true, title: "Home pushes the agent's commits to this pull request's branch (you confirm first; never forced)" });
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
  const ref = refOf(page, ctx);
  const repoFolder = !!ctx.folder && ctx.folder.relation !== "fork";
  const noFolder = "Add this repository to Launchpad first";
  out.push({ label: "Ask a bot…", hint: "A bot of the workspace, with this item as context", onSelect: () => askPopover(root, anchor, page, ctx, null, "ask", refresh) });
  if (page.kind !== "issue") out.push({ label: "Review with agent", hint: "A suggested review for you to post", onSelect: () => askPopover(root, anchor, page, ctx, null, "review", refresh) });
  else {
    out.push({ label: "Draft a reply", hint: "A suggested comment for you to post", onSelect: () => askPopover(root, anchor, page, ctx, null, "reply", refresh) });
    out.push({ label: "Investigate", hint: repoFolder ? "A read-only agent finds the root cause" : noFolder, disabled: !repoFolder, onSelect: () => launchPopover(root, anchor, ctx, "investigate", ref, refresh) });
    out.push({ label: "Fix in a Local PR", hint: repoFolder ? "An agent fixes it and opens a Local PR linked to this issue" : noFolder, disabled: !repoFolder, onSelect: () => launchPopover(root, anchor, ctx, "fix", ref, refresh) });
    const latest = agents.find((a) => a.bot_id);
    if (latest) out.push({ label: `Continue with ${who(latest)}`, hint: "Message its bot about this issue", onSelect: () => askPopover(root, anchor, page, ctx, null, "ask", refresh, latest.bot_id) });
  }
  const ready = page.kind === "issue" ? [] : ctx.branches.filter((b) => b.push_ready);
  for (const b of ready) {
    out.push({ label: `Push ${b.branch} to this PR`, hint: canPush(ctx) ? `${b.push_ready!.ahead} commit${b.push_ready!.ahead === 1 ? "" : "s"}; you confirm first` : "Maintainers cannot edit this pull request's branch", disabled: !canPush(ctx), onSelect: () => pushDialog(root, anchor, page, ctx, b, refresh) });
    if (b.push_ready?.session_id) out.push({ label: "View its changes", hint: "The agent's branch in Launchpad", onSelect: () => open(agentPath({ session_id: b.push_ready!.session_id! }), ws) });
  }
  if (review) out.push({ label: "Open the suggested review", hint: "In Launchpad, with its line comments", onSelect: () => open(`/github/${page.repo}/${page.number}?tab=files&draft=${encodeURIComponent(review)}`, ws) });
  for (const a of agents.slice(0, 5)) {
    out.push({ separator: true });
    out.push({ label: `Show ${who(a)}'s work`, hint: `${a.title}: its turns and steps, here on the page`, onSelect: () => openAgentView({ session_id: a.session_id, title: a.title, bot: a.bot, running: running(a) }, ws) });
    out.push({ label: `Open ${who(a)} in Launchpad`, hint: a.title, onSelect: () => open(agentPath(a), ws) });
    if (a.status === "working" || a.status === "starting") out.push({ label: `Stop ${who(a)}`, hint: "Stops its turn", onSelect: () => void stop(a).then(refresh).catch((e) => console.warn("Launchpad: stop", e)) });
  }
  for (const p of ctx.local_prs.slice(0, 3)) out.push({ label: `Open Local PR ${p.title}`, hint: p.state, onSelect: () => open(`/predrafts/${encodeURIComponent(p.id)}`, ws) });
  out.push({ separator: true });
  out.push({ label: "Open in Launchpad", hint: "This item in Launchpad", onSelect: () => open(`/github/${page.repo}/${page.number}`, ws) });
  return out;
}

/** Home may push to this pull request's branch: the same repository, or a fork whose author lets maintainers edit (or
 * the owner's own fork). Unknown (no cached item yet): let Home decide when pushing. */
function canPush(ctx: ExtContext) {
  const head = ctx.page.head_repo;
  if (!head || !ctx.page.repo || head.toLowerCase() === ctx.page.repo.toLowerCase()) return true;
  if (ctx.login && head.split("/")[0]?.toLowerCase() === ctx.login.toLowerCase()) return true;
  return ctx.page.maintainer_can_modify !== false;
}

/** The Branch rows: commits ready to push, not on GitHub yet (with the last error), and a fork's edit permission. */
function branchState(ctx: ExtContext, branches: ExtBranch[]): HTMLElement[] {
  const rows: HTMLElement[] = [];
  const fork = !!ctx.page.head_repo && !!ctx.page.repo && ctx.page.head_repo.toLowerCase() !== ctx.page.repo.toLowerCase();
  for (const b of branches.filter((x) => x.push_ready)) {
    const n = b.push_ready!.ahead;
    const target = ctx.page.head_branch ? `${fork ? `${ctx.page.head_repo!.split("/")[0]}:` : ""}${ctx.page.head_branch}` : "this pull request's branch";
    rows.push(h("div", { class: "lp-row", title: `${b.branch} at ${b.head?.slice(0, 7)}` }, h("span", { class: "lp-dot lp-need" }), h("span", { class: "lp-grow", text: `${n} commit${n === 1 ? "" : "s"} ready to push` })));
    rows.push(h("div", { class: "lp-muted", text: `${b.branch} is ${n} ahead of ${target}${b.push_ready!.title ? ` (${b.push_ready!.title})` : ""}.` }));
  }
  for (const b of branches.filter((x) => !x.push_ready && (x.not_on_github || x.error))) {
    rows.push(h("div", { class: "lp-row", title: b.error ?? undefined }, h("span", { class: b.error ? "lp-dot lp-bad" : "lp-dot lp-need" }), h("span", { class: "lp-grow", text: "Not on GitHub yet" })));
    rows.push(h("div", { class: "lp-muted", text: b.error ? `Push of ${b.branch} refused: ${b.error.slice(0, 200)}${b.head ? ` Commit ${b.head.slice(0, 7)}.` : ""}` : `${b.branch}: Home pushes it to GitHub next.` }));
  }
  if (fork && ctx.page.maintainer_can_modify !== null) {
    const can = ctx.page.maintainer_can_modify || canPush(ctx);
    rows.push(h("div", { class: "lp-row" }, h("span", { class: can ? "lp-dot lp-ok" : "lp-dot lp-bad" }), h("span", { class: "lp-grow", text: can ? "Maintainers can edit" : "Maintainers cannot edit" })));
    if (!can) rows.push(h("div", { class: "lp-muted", text: "Agents cannot push here: Continue makes a Local PR on top of this branch, or a review with suggestion blocks." }));
  }
  return rows;
}

/** Push to this PR: the extension's own confirmation naming the branch, its commit and the target; only a real click
 * of its button pushes (`branch.push-to-pr`), and Home checks again (open, fast-forward, never forced). */
function pushDialog(root: ShadowRoot, anchor: HTMLElement, page: ItemPage, ctx: ExtContext, b: ExtBranch, refresh: () => void) {
  const ready = b.push_ready!;
  const error = h("div", { class: "lp-error", attrs: { role: "alert", hidden: "" } });
  const go = h("button", { class: "lp-btn lp-primary", text: "Push to this PR", attrs: { type: "button" } });
  const target = ctx.page.head_repo && ctx.page.head_branch ? `${ctx.page.head_repo}:${ctx.page.head_branch}` : "this pull request's branch";
  const fork = !!ctx.page.head_repo && ctx.page.head_repo.toLowerCase() !== page.repo.toLowerCase();
  let busy = false;
  const close = popover(root, anchor, `Push to ${page.repo}#${page.number}`, (close) => [
    h("div", { class: "lp-confirm", text: `Home pushes ${ready.branch} at ${ready.head.slice(0, 8)} (${ready.ahead} commit${ready.ahead === 1 ? "" : "s"} on top of the pull request's head) to ${target}${fork ? ", the author's fork" : ""} on GitHub. Fast-forward only, never forced; refused if the pull request moved. Then Home keeps pushing the agent's new commits there.` }),
    error,
    h("div", { class: "lp-row" }, h("span", { class: "lp-grow" }), button("Cancel", close, { small: true }), go),
  ]);
  go.addEventListener("click", (e) => {
    e.preventDefault();
    // Only a real click of the owner pushes (never a click the page synthesized).
    if (!e.isTrusted || busy) return;
    busy = true;
    go.disabled = true;
    go.textContent = "Pushing…";
    error.hidden = true;
    request("branch.push-to-pr", { repo: page.repo, number: page.number, branch: ready.branch, head: ready.head })
      .then(() => { close(); refresh(); })
      .catch((err) => { error.textContent = err instanceof Error ? err.message : String(err); error.hidden = false; busy = false; go.disabled = false; go.textContent = "Push to this PR"; });
  });
}
