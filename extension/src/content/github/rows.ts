// Row states of list pages (Pull requests, Issues): one `rows` request for the visible rows, refetched when the set of
// rows changes or Home's records change (`ext.context`), and the row chip: at most one per row, by priority: needs you
// (amber) › working (blue) › a bot's report (green) › commits to push. A row's
// hover action shows only while the pointer is on the row (or it has focus); it never replaces GitHub's controls.

import type { ExtRow } from "@/gen/ExtRow";
import type { ExtRows } from "@/gen/ExtRows";
import { request } from "../../shared/messages";
import { agentPath, elapsed, open, running, suggestionPath, who } from "./item";
import { h, type Widget } from "./widgets";

export type Chip = { text: string; tone: "need" | "run" | "ok" | "quiet"; title: string; go: () => void };

const OUTCOME: Record<string, string> = { merged: "merged", approved: "approved", commented: "answered", closed: "closed", waiting: "waiting", info: "reported", needs_you: "asks you" };

/** The one chip of a row, or null. */
export function chipOf(row: ExtRow, ws: string | null): Chip | null {
  const repo = row.repo ?? "";
  const n = row.number;
  const asks = row.agents.find((a) => a.open_question);
  if (asks) return { text: `${who(asks)} asks you`, tone: "need", title: asks.title, go: () => open(agentPath(asks), ws) };
  const review = row.suggestions.find((s) => s.kind === "review" && s.status === "draft");
  if (review && n !== null) return { text: `Review ready${review.comments ? ` · ${review.comments} comment${review.comments === 1 ? "" : "s"}` : ""}`, tone: "need", title: `${review.bot ?? "An agent"} wrote a review for you to post: its card in Launchpad's Today posts it`, go: () => open(suggestionPath(review.id), ws) };
  const reply = row.suggestions.find((s) => s.kind === "comment" && s.status === "draft");
  if (reply && n !== null) return { text: `Reply ready${reply.bot ? ` · ${reply.bot}` : ""}`, tone: "need", title: "A suggested comment: open the item and Insert it", go: () => { location.assign(`https://github.com/${repo}/issues/${n}`); } }; // GitHub sends /issues/<n> of a pull request to its page.
  const report = row.report;
  if (report?.outcome === "needs_you" && report.waiting) return { text: `${report.bot ?? "A bot"} asks you`, tone: "need", title: report.title, go: () => open(`/?card=${encodeURIComponent(`proposal:${report.id}`)}`, ws) };
  const done = row.agents.find((a) => a.needs_you);
  if (done) return { text: `${who(done)} · done`, tone: "need", title: done.title, go: () => open(agentPath(done), ws) };
  const working = row.agents.find(running);
  if (working) return { text: `${who(working)} working · ${elapsed(working.since)}`, tone: "run", title: working.title, go: () => open(agentPath(working), ws) };
  if (report) return { text: `${report.bot ?? "A bot"} · ${OUTCOME[report.outcome] ?? report.outcome}`, tone: ["merged", "approved", "commented", "closed"].includes(report.outcome) ? "ok" : "quiet", title: report.title, go: () => open(`/?card=${encodeURIComponent(`proposal:${report.id}`)}`, ws) };
  if (row.push) { const p = row.push; return { text: "Commits to push", tone: "need", title: p.error ? `Home's last push failed: ${p.error}` : `${p.branch}: Home has commits GitHub does not have yet`, go: () => { location.assign(`https://github.com/${repo}/pull/${n}`); } }; }
  return null;
}

/** The chip as a button in a widget. */
export function chipButton(c: Chip) {
  const b = h("button", { class: `lp-tag lp-${c.tone}`, title: c.title, attrs: { type: "button" } }, h("span", { text: c.text }));
  b.addEventListener("click", (e) => { e.preventDefault(); e.stopPropagation(); c.go(); });
  return b;
}

/** Shows a widget's `.lp-hover` part only while the pointer is on its row, or the row has focus. */
const hovered = new WeakSet<Element>();
export function hoverWith(row: Element, w: Widget) {
  w.host.setAttribute("data-hover", "");
  if (row.matches(":hover")) w.host.setAttribute("data-show", "");
  if (hovered.has(row)) return;
  hovered.add(row);
  const host = () => row.querySelector<HTMLElement>(":scope launchpad-mark[data-hover]");
  const show = (on: boolean) => { const el = host(); if (el) el.toggleAttribute("data-show", on || row.matches(":focus-within")); };
  row.addEventListener("pointerenter", () => show(true));
  row.addEventListener("pointerleave", () => show(false));
  row.addEventListener("focusin", () => show(true));
  row.addEventListener("focusout", () => setTimeout(() => show(row.matches(":hover")), 0));
}

/** The rows of the visible items: fetched once per set of items, again after `stale()`. */
export class RowStates {
  private key = "";
  private data: ExtRows | null = null;
  private pending: string | null = null;
  constructor(private changed: () => void) {}
  /** The states for `items` of page `url` (null until Home answered); asks Home when the set changed. */
  get(url: string, items: string[]): ExtRows | null {
    const key = `${url} ${[...items].sort().join(",")}`;
    if (key !== this.key && key !== this.pending && items.length) {
      this.pending = key;
      void request("rows", { url, items }).then((data) => {
        if (this.pending !== key) return;
        this.pending = null;
        this.key = key;
        this.data = data;
        this.changed();
      }).catch(() => { if (this.pending === key) this.pending = null; });
    }
    // Until Home answers for a new set, the rows it already answered keep their chips (matched by item).
    return this.data;
  }
  row(item: string): ExtRow | null {
    return this.data?.rows.find((r) => r.item === item) ?? null;
  }
  /** Home's records changed: ask again at the next render. */
  stale() {
    this.key = "";
  }
  clear() {
    this.key = "";
    this.data = null;
    this.pending = null;
  }
}
