// The agent's work, read on the GitHub page: a drawer on the right with what an agent of this item was asked, its
// answer per turn (Markdown) and its steps (titles only, folded), built from the session's events like the bot chat
// (`chatRows`). It refreshes every 5 s while open. Read only: Stop and the full trace stay in Launchpad (Open in
// Launchpad). Text comes from Home, never from the page.

import type { SessionEvent } from "@/gen/SessionEvent";
import { chatRows, duration, type ChatRow } from "../../panel/thread";
import { request } from "../../shared/messages";
import { agentPath, open } from "./item";
import { markdown } from "./markdown";
import { button, h, logo, mount, unmount } from "./widgets";

const ID = "agent-view";
const STYLE_ID = "lp-drawer-style";

export type ViewedAgent = { session_id: string; title?: string | null; bot?: string | null; running?: boolean };

let current: { agent: ViewedAgent; ws: string | null; timer: ReturnType<typeof setInterval> | null; open: Set<string>; onKey: (e: KeyboardEvent) => void } | null = null;

const DRAWER = `
.lp-drawer { position: fixed; z-index: 2147482000; top: 0; right: 0; bottom: 0; width: min(560px, 100vw); display: flex; flex-direction: column;
  background: var(--overlay-bgColor, var(--bgColor-default, #ffffff)); border-left: 1px solid var(--borderColor-default, #d1d9e0);
  box-shadow: var(--shadow-floating-large, 0 8px 24px rgba(140, 149, 159, .2)); font-size: 13px; font-weight: 400; }
.lp-drawer-head { display: flex; align-items: center; gap: 8px; padding: 12px 12px 10px 16px; border-bottom: 1px solid var(--borderColor-muted, #d1d9e0b3); border-top: 2px solid var(--lp-teal); }
.lp-drawer-body { flex: 1; overflow: auto; padding: 12px 16px 24px; display: flex; flex-direction: column; gap: 10px; }
.lp-turn-user { align-self: flex-end; max-width: 88%; padding: 8px 10px; border-radius: 10px; background: var(--bgColor-muted, #f6f8fa); white-space: pre-wrap; word-break: break-word; font-size: 13px; }
.lp-turn-user.lp-folded { max-height: 9em; overflow: hidden; -webkit-mask-image: linear-gradient(180deg, #000 70%, transparent); mask-image: linear-gradient(180deg, #000 70%, transparent); cursor: pointer; }
.lp-from { align-self: flex-end; font-size: 12px; color: var(--fgColor-muted, #59636e); margin-bottom: -6px; }
.lp-steps summary { cursor: pointer; color: var(--fgColor-muted, #59636e); font-size: 12px; }
.lp-steps ol { margin: 6px 0 0; padding-left: 20px; color: var(--fgColor-muted, #59636e); font-size: 12px; }
.lp-steps li { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.lp-ended { font-size: 12px; color: var(--fgColor-danger, #d1242f); }
`;

/** Opens (or switches) the drawer on `agent`. `ws`: the workspace, for Open in Launchpad. */
export function openAgentView(agent: ViewedAgent, ws: string | null) {
  closeAgentView();
  const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") { e.stopPropagation(); closeAgentView(); } };
  current = { agent, ws, timer: null, open: new Set(), onKey };
  document.addEventListener("keydown", onKey, true);
  const w = mount(ID, agent.session_id, document.body, "beforeend");
  w.root.append(h("style", { attrs: { id: STYLE_ID }, text: DRAWER }));
  const body = h("div", { class: "lp-drawer-body", attrs: { "aria-live": "polite" } }, h("div", { class: "lp-muted", text: "Loading…" }));
  const title = agent.title || "Agent";
  const drawer = h("div", { class: "lp-drawer", attrs: { role: "dialog", "aria-label": `${agent.bot ?? "Agent"}: ${title}` } },
    h("div", { class: "lp-drawer-head" }, logo(),
      h("span", { class: "lp-grow" }, h("span", { class: "lp-title", text: agent.bot ?? "Agent" }), h("span", { class: "lp-muted", text: ` · ${title}` })),
      button("Open in Launchpad", () => open(agentPath(agent), ws), { small: true, title: "Its full trace, Stop and its thread" }),
      button("Close", closeAgentView, { small: true })),
    body);
  w.root.append(drawer);
  const load = () => {
    const mine = current;
    request("session.events", { id: agent.session_id, tail: 400 })
      .then((p) => { if (current === mine) draw(body, p.events); })
      .catch((e) => { if (current === mine) body.replaceChildren(h("div", { class: "lp-error", text: e instanceof Error ? e.message : String(e) })); });
  };
  load();
  current.timer = setInterval(() => { if (document.visibilityState === "visible") load(); }, 5_000);
}

export function closeAgentView() {
  if (!current) return;
  if (current.timer) clearInterval(current.timer);
  document.removeEventListener("keydown", current.onKey, true);
  current = null;
  unmount(ID);
}

/** Running: the last turn did not end (an agent's events after its last `turn_end`). */
function runningOf(events: SessionEvent[]) {
  const last = events.findLastIndex((e) => e.body.kind === "turn_end");
  return events.slice(last + 1).some((e) => e.body.kind !== "user_message") || (last < 0 && events.length > 0);
}

let drawn = "";
function draw(body: HTMLElement, events: SessionEvent[]) {
  const state = current;
  if (!state) return;
  const running = state.agent.running ?? runningOf(events);
  const rows = chatRows(events, running, {});
  const key = JSON.stringify([state.agent.session_id, events.at(-1)?.seq, running, rows.length]);
  if (key === drawn && body.childElementCount) return;
  drawn = key;
  const atEnd = body.scrollHeight - body.scrollTop - body.clientHeight < 40;
  const scroll = body.scrollTop;
  body.replaceChildren(...(rows.length ? rows.map((r) => row(r, state.open)) : [h("div", { class: "lp-muted", text: "No messages yet." })]));
  if (events.length >= 400) body.prepend(h("div", { class: "lp-muted", text: "Earlier turns are in Launchpad (Open in Launchpad)." }));
  body.scrollTop = atEnd ? body.scrollHeight : scroll;
}

function row(r: ChatRow, opened: Set<string>): HTMLElement {
  switch (r.t) {
    case "user": {
      const text = h("div", { class: `lp-turn-user${r.text.length > 400 ? " lp-folded" : ""}`, text: r.text, title: r.text.length > 400 ? "Show all" : undefined });
      if (r.text.length > 400) text.addEventListener("click", () => text.classList.remove("lp-folded"), { once: true });
      return r.from ? h("div", { attrs: { style: "display:flex;flex-direction:column;gap:8px" } }, h("div", { class: "lp-from", text: r.from }), text) : text;
    }
    case "answer": return markdown(r.text);
    case "steps": {
      const d = h("details", { class: "lp-steps" },
        h("summary", { text: `${r.count} step${r.count === 1 ? "" : "s"}${r.durationMs ? ` · ${duration(r.durationMs)}` : ""}` }),
        h("ol", {}, ...r.titles.map((t) => h("li", { text: t, title: t }))));
      d.open = opened.has(r.key);
      d.addEventListener("toggle", () => { if (d.open) opened.add(r.key); else opened.delete(r.key); });
      return d;
    }
    case "ask": return h("div", { class: "lp-row" }, h("span", { class: "lp-dot lp-need" }), h("span", { text: `Asks you: ${r.question}` }));
    case "ended": return h("div", { class: "lp-ended", text: r.text });
    case "working": return h("div", { class: "lp-row" }, h("span", { class: "lp-dot lp-run" }), h("span", { class: "lp-muted", text: r.step ? `Working · ${r.step}` : "Working…" }));
  }
}
