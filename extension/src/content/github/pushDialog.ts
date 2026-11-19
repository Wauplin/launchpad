// Push to this PR: the extension's own confirmation naming the branch and its commit; only a real click of its button
// pushes (`branch.push-to-pr`), and Home checks on GitHub at that click (open, may push there, fast-forward, never
// forced). A push that runs an agent's ready card starts that card afterwards (the worker's `/ran`). One push at a time
// per branch and commit in this tab, whichever widget (sidebar box, timeline) started it.

import { request } from "../../shared/messages";
import type { ItemPage } from "./item";
import { pushKey, type PushTarget } from "../../shared/readyPush";
import { button, h, popover } from "./widgets";

const inFlight = new Set<string>();
/** Pushes done in this tab (key → what the timeline says) until Home's context shows them. */
const done = new Map<string, string>();

export const pushing = (t: PushTarget) => inFlight.has(pushKey(t));
export const pushedNow = (t: PushTarget) => done.get(pushKey(t)) ?? null;

export function pushDialog(root: ShadowRoot, anchor: HTMLElement, page: ItemPage, t: PushTarget, refresh: () => void) {
  const key = pushKey(t);
  const error = h("div", { class: "lp-error", attrs: { role: "alert", hidden: "" } });
  const go = h("button", { class: "lp-btn lp-primary", text: "Push to this PR", attrs: { type: "button" } });
  if (inFlight.has(key)) { go.disabled = true; go.textContent = "Pushing…"; }
  const commits = t.ahead === null ? "" : ` (${t.ahead} commit${t.ahead === 1 ? "" : "s"} since its agent started)`;
  const card = t.card ? " This runs the push the agent prepared; its card in Today is then done." : "";
  const close = popover(root, anchor, `Push to ${page.repo}#${page.number}`, (close) => [
    h("div", { class: "lp-confirm", text: `Home pushes ${t.branch} at ${t.head.slice(0, 8)}${commits} to this pull request's branch on GitHub. Fast-forward only, never forced; refused if the pull request moved or Launchpad may not push there. Then Home keeps pushing the agent's new commits there.${card}` }),
    error,
    h("div", { class: "lp-row" }, h("span", { class: "lp-grow" }), button("Cancel", close, { small: true }), go),
  ]);
  go.addEventListener("click", (e) => {
    e.preventDefault();
    // Only a real click of the owner pushes (never a click the page synthesized), once at a time.
    if (!e.isTrusted || inFlight.has(key)) return;
    inFlight.add(key);
    go.disabled = true;
    go.textContent = "Pushing…";
    error.hidden = true;
    request("branch.push-to-pr", { repo: page.repo, number: page.number, branch: t.branch, head: t.head, ...(t.card ? { card: t.card } : {}) })
      .then((r) => {
        done.set(key, r.card_error ? `Pushed ${t.head.slice(0, 7)} to this pull request; its card in Today is still waiting (${r.card_error}).` : `Pushed ${t.head.slice(0, 7)} to this pull request.`);
        inFlight.delete(key);
        close();
        refresh();
      })
      .catch((err) => {
        inFlight.delete(key);
        error.textContent = err instanceof Error ? err.message : String(err);
        error.hidden = false;
        go.disabled = false;
        go.textContent = "Push to this PR";
      });
  });
}
