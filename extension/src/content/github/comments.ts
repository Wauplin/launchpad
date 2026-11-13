// I2 Ask a bot on each comment: a small "Ask" in the comment's header actions, highlighted ("Ask <bot> what to
// answer", amber) on comments that mention you or on a pull request that requests your review. I5 the live agent card
// under the comment that was asked about: working with its current step and time, Stop and Open in Launchpad; then
// "Reply ready" with the suggestion's preview and Insert, from the event stream. The comment is passed by reference
// (its anchor ID); its text is never read.

import type { ExtAgent } from "@/gen/ExtAgent";
import type { ExtContext } from "@/gen/ExtContext";
import { commentBody, commentOf, commentRoot, findAnchors } from "./anchors";
import { askPopover } from "./askPopover";
import { insertSuggestion, NeedsChoice } from "./insert";
import { agentOf, agentPath, dotClass, elapsed, open, replyFor, running, statusText, stop, who, type ItemPage } from "./item";
import { stepOfSession, watch } from "./live";
import { button, h, logo, more, mount, unmount } from "./widgets";

/** Does the comment mention the owner (`@login` link in its body)? Only the link's target is compared. */
function mentions(root: Element, login: string | null | undefined) {
  if (!login) return false;
  const body = commentBody(root);
  if (!body) return false;
  const want = `/${login}`.toLowerCase();
  return Array.from(body.querySelectorAll("a[href]")).some((a) => {
    try { return new URL(a.getAttribute("href") ?? "", location.origin).pathname.toLowerCase() === want && (a.textContent ?? "").trim().startsWith("@"); } catch { return false; }
  });
}

export function renderComments(page: ItemPage, ctx: ExtContext | null, refresh: () => void) {
  const found = ctx ? findAnchors("comments", { repo: page.repo, number: page.number }) : null;
  const ids = new Set<string>();
  const cards = new Set<string>();
  const live: string[] = [];
  const reviewRequested = ctx?.inbox?.reason === "review_requested";
  for (const actions of found?.els ?? []) {
    const c = commentOf(actions);
    if (!c || !ctx) continue;
    const id = `ask-${c.id}`;
    ids.add(id);
    const highlight = mentions(c.root, ctx.login) || (reviewRequested && c.id.startsWith("issuecomment-") && c.root === document.querySelector('.timeline-comment-group[id^="issuecomment-"], [data-testid="comment-header"][id^="issuecomment-"]'));
    const bot = ctx.defaults.bot;
    const text = highlight ? `Ask ${bot?.name ?? "a bot"} what to answer` : "Ask";
    const w = mount(id, `${c.id}|${text}`, actions, found!.place);
    if (w.fresh) {
      const chip = h("button", {
        class: `lp-chip ${highlight ? "lp-accent lp-mention" : "lp-quiet"}`,
        title: highlight ? "This comment mentions you: ask a bot what to answer" : "Ask a bot about this comment",
        attrs: { type: "button", "aria-haspopup": "dialog" },
        on: { click: (e) => { e.preventDefault(); e.stopPropagation(); askPopover(w.root, chip, page, ctx, c.id, "ask", refresh); } },
      }, logo(), h("span", { class: "lp-text", text }));
      w.root.append(chip);
    }
    // The live card of the agent asked about this comment.
    const agent = agentOf(ctx, c.id);
    if (agent) {
      cards.add(`card-${c.id}`);
      if (running(agent)) live.push(agent.session_id);
      renderCard(page, ctx, c.id, agent, refresh);
    }
  }
  for (const el of document.querySelectorAll<HTMLElement>('[data-launchpad^="ask-"]')) if (!ids.has(el.dataset.launchpad ?? "")) el.remove();
  for (const el of document.querySelectorAll<HTMLElement>('[data-launchpad^="card-"]')) if (!cards.has(el.dataset.launchpad ?? "")) el.remove();
  return live;
}

function renderCard(page: ItemPage, ctx: ExtContext, comment: string, a: ExtAgent, refresh: () => void) {
  const id = `card-${comment}`;
  const root = commentRoot(comment);
  if (!root) return unmount(id);
  const ws = ctx.folder?.workspace_id ?? ctx.workspace?.id ?? null;
  const reply = replyFor(ctx, comment, a);
  const step = stepOfSession(a.session_id);
  // The minute counts in the key: the card re-renders when its time changes.
  const key = `${a.session_id}|${a.status}|${step ?? ""}|${reply?.id ?? ""}:${reply?.status ?? ""}|${elapsed(a.since)}|${a.last_message ?? ""}`;
  const w = mount(id, key, root, "afterend", "card");
  if (!w.fresh) return;
  const error = h("div", { class: "lp-error", attrs: { role: "alert", hidden: "" } });
  const fail = (e: unknown) => { error.textContent = e instanceof NeedsChoice ? "GitHub's box already has text: use Insert on the banner over the box." : e instanceof Error ? e.message : String(e); error.hidden = false; };
  const head = h("div", { class: "lp-row" }, h("span", { class: dotClass(a) }), logo(), h("span", { class: "lp-title", text: who(a) }));
  const card = h("div", { class: "lp-card" }, head);
  const openIt = () => open(agentPath(a), ws);
  if (running(a)) {
    head.append(h("span", { class: "lp-muted lp-grow", text: `${statusText(a)} · ${elapsed(a.since)}` }));
    card.append(h("div", { class: "lp-muted", text: step || "Starting…", attrs: { style: "margin-top:4px" } }));
    card.append(h("div", { class: "lp-actions" }, button("Stop", () => void stop(a).then(refresh).catch(fail), { small: true }), button("Open in Launchpad", openIt, { small: true })));
  } else if (reply) {
    head.append(h("span", { class: "lp-muted lp-grow", text: reply.status === "inserted" ? "Reply inserted" : "Reply ready" }));
    card.append(h("div", { class: "lp-preview", text: reply.text }));
    const actions = h("div", { class: "lp-actions" });
    if (reply.status === "draft") actions.append(button("Insert", () => void insertSuggestion(page, reply).catch(fail), { primary: true, small: true, title: "Puts the reply into GitHub's box: you edit it and post it with GitHub's button" }));
    actions.append(button("Open in Launchpad", openIt, { small: true }));
    card.append(actions);
  } else {
    head.append(h("span", { class: "lp-muted lp-grow", text: `${statusText(a)} · ${elapsed(a.since)}` }));
    if (a.last_message) card.append(h("div", { class: "lp-preview", text: a.last_message }));
    card.append(h("div", { class: "lp-actions" }, button("Open in Launchpad", openIt, { small: true }), more(w.root, "Agent", () => [
      { label: "Ask again with a note", hint: `${who(a)}, about this comment`, onSelect: () => askPopover(w.root, head, page, ctx, comment, "ask", refresh, a.bot_id) },
    ])));
  }
  card.append(error);
  w.root.append(card);
}

export { watch };
