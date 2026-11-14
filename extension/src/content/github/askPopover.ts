// The Ask popover (I2, and the box's main action): a bot of the workspace (the page's default first: the folder's bot,
// else the last one used on the repository, else the Ask target), an optional note, Send. The bot gets the item or
// the comment by reference.

import type { ExtContext } from "@/gen/ExtContext";
import type { BotBrief } from "../../shared/messages";
import { ask, itemTitle, workspaceBots, type ItemPage } from "./item";
import { button, h, popover } from "./widgets";

export type AskKind = "ask" | "review" | "reply";

const TITLES: Record<AskKind, string> = { ask: "Ask a bot about this comment", review: "Review with agent", reply: "Draft a reply" };

/** Opens the popover under `anchor`. `prefer`: the bot to choose first (an agent's bot for Continue). */
export function askPopover(root: ShadowRoot, anchor: HTMLElement, page: ItemPage, ctx: ExtContext | null, comment: string | null, kind: AskKind, done: () => void, prefer?: string | null) {
  const select = h("select", { attrs: { "aria-label": "Bot" } }, h("option", { text: "Loading bots…", attrs: { value: "" } }));
  const note = h("textarea", { attrs: { "aria-label": "Note for the bot (optional)", placeholder: kind === "review" ? "What to look at (optional)" : "Your note (optional): what you think, what to check" } });
  const title = itemTitle(page);
  const what = h("div", { class: "lp-muted", text: comment ? `${page.repo}#${page.number} · ${comment.startsWith("discussion_r") ? "review comment" : "comment"} ${comment.replace(/^\D+/, "")}` : `${page.repo}#${page.number}${title ? ` · ${title}` : ""}` });
  const error = h("div", { class: "lp-error", attrs: { role: "alert", hidden: "" } });
  let bots: BotBrief[] = [];
  let busy = false;
  const send = h("button", { class: "lp-btn lp-primary", text: "Send", attrs: { type: "button", disabled: "" } });
  const close = popover(root, anchor, TITLES[kind], (close) => [
    what,
    select,
    note,
    error,
    h("div", { class: "lp-row" }, h("span", { class: "lp-muted lp-grow", text: ctx?.folder ? `Folder ${ctx.folder.name} · ${ctx.folder.workspace}` : "Not in Launchpad: a temporary folder" }), button("Cancel", close, { small: true }), send),
  ]);
  const fail = (e: unknown) => {
    error.textContent = e instanceof Error ? e.message : String(e);
    error.hidden = false;
  };
  const submit = async () => {
    const bot = bots.find((b) => b.id === select.value);
    if (!bot || busy) return;
    busy = true;
    send.disabled = true;
    send.textContent = "Sending…";
    error.hidden = true;
    try {
      await ask(page, bot, note.value, comment, kind);
      close();
      done();
    } catch (e) {
      fail(e);
      busy = false;
      send.disabled = false;
      send.textContent = "Send";
    }
  };
  send.addEventListener("click", (e) => { e.preventDefault(); void submit(); });
  // Enter sends, Shift+Enter adds a line (as in Launchpad's message boxes).
  note.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing) { e.preventDefault(); void submit(); }
    e.stopPropagation(); // GitHub's own shortcuts stay out of our box.
  });
  void workspaceBots().then(({ bots: list, ask: target }) => {
    bots = list.filter((b) => !b.paused);
    select.replaceChildren(...bots.map((b) => h("option", { text: b.name, attrs: { value: b.id } })));
    if (!bots.length) { fail("No bot in this workspace: create one in Launchpad (Bots → New bot)."); return; }
    const first = [prefer, ctx?.defaults.bot?.id, target].find((id) => id && bots.some((b) => b.id === id));
    if (first) select.value = first;
    send.disabled = false;
    note.focus();
  }).catch(fail);
}
