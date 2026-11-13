// Private notes (`launchpad note`) under the description of a pull request or issue: what the workspace's agents want
// the owner to know about the item before acting on it. Only the owner sees them: they live on Home and are drawn in a
// closed shadow root, never written into GitHub's page or fields. Each note shows its author, its age, its Markdown
// (folded when long) and Show work, which opens the agent's turns on the page (agentView.ts); ⋯ holds Open in
// Launchpad and Delete note.

import type { ExtContext } from "@/gen/ExtContext";
import type { ExtNote } from "@/gen/ExtNote";
import { request } from "../../shared/messages";
import { openAgentView } from "./agentView";
import { findAnchor } from "./anchors";
import { agentPath, elapsed, open, type ItemPage } from "./item";
import { markdown } from "./markdown";
import { button, h, logo, more, mount, popover, unmount } from "./widgets";

const ID = "item-notes";
/** Notes unfolded in this tab. */
const unfolded = new Set<string>();

export function renderNotes(page: ItemPage, ctx: ExtContext | null, refresh: () => void) {
  const notes = ctx?.notes ?? [];
  if (!ctx || !notes.length) return unmount(ID);
  const anchor = findAnchor("item-description", { repo: page.repo, number: page.number });
  if (!anchor) return unmount(ID);
  const key = JSON.stringify([page.url, notes.map((n) => `${n.id}:${n.updated_at}`), [...unfolded]]);
  const w = mount(ID, key, anchor.el, anchor.place, "card");
  if (!w.fresh) return;
  const ws = ctx.folder?.workspace_id ?? ctx.workspace?.id ?? null;
  const card = h("div", { class: "lp-card" },
    h("div", { class: "lp-row" }, logo(), h("span", { class: "lp-title", text: notes.length === 1 ? "Private note" : `${notes.length} private notes` }), h("span", { class: "lp-muted lp-grow", text: "· only you see this, never posted" })));
  for (const n of notes) card.append(note(w.root, n, ctx, ws, refresh));
  w.root.append(card);
}

function note(root: ShadowRoot, n: ExtNote, ctx: ExtContext, ws: string | null, refresh: () => void) {
  const agent = n.session_id ? ctx.agents.find((a) => a.session_id === n.session_id) ?? null : null;
  const long = n.body.length > 1500 && !unfolded.has(n.id);
  const body = markdown(n.body);
  if (long) body.setAttribute("style", "max-height: 22em; overflow: hidden; -webkit-mask-image: linear-gradient(180deg, #000 75%, transparent); mask-image: linear-gradient(180deg, #000 75%, transparent)");
  const head = h("div", { class: "lp-row" },
    h("span", { class: "lp-grow" }, h("strong", { text: n.author }), h("span", { class: "lp-muted", text: ` · ${elapsed(n.updated_at)}${elapsed(n.updated_at) === "now" ? "" : " ago"}` })));
  if (n.session_id) {
    const sid = n.session_id;
    head.append(button("Show work", () => openAgentView({ session_id: sid, title: agent?.title, bot: agent?.bot ?? n.author, running: agent ? ["working", "starting", "background"].includes(agent.status) : undefined }, ws), { small: true, title: "What it was asked, its answers and its steps, here on the page" }));
  }
  const menu: HTMLElement = more(root, "Note", () => [
    ...(n.session_id ? [{ label: "Open the agent in Launchpad", hint: "Its full trace and thread", onSelect: () => open(agentPath({ session_id: n.session_id! }), ws) }] : []),
    { label: "Delete note…", hint: "Removes it from Launchpad", onSelect: () => confirmDelete(root, menu, n, refresh) },
  ]);
  head.append(menu);
  const out = h("div", { class: "lp-sect", attrs: { style: "margin-top: 8px; padding-top: 8px; border-top: 1px solid var(--borderColor-muted, #d1d9e0b3)" } }, head, h("div", { attrs: { style: "margin-top: 6px" } }, body));
  if (long) out.append(h("div", { class: "lp-actions" }, button("Show all", () => { unfolded.add(n.id); refresh(); }, { small: true })));
  return out;
}

function confirmDelete(root: ShadowRoot, anchor: HTMLElement, n: ExtNote, refresh: () => void) {
  const error = h("div", { class: "lp-error", attrs: { role: "alert", hidden: "" } });
  popover(root, anchor, "Delete note", (close) => [
    h("div", { text: `Deletes ${n.author}'s note about this item. The agent may write a new one later.` }),
    error,
    h("div", { class: "lp-row" }, h("span", { class: "lp-grow" }), button("Cancel", close, { small: true }), button("Delete", () => {
      request("note.delete", { id: n.id }).then(() => { close(); refresh(); }).catch((e) => { error.textContent = e instanceof Error ? e.message : String(e); error.hidden = false; });
    }, { small: true, primary: true })),
  ]);
}
