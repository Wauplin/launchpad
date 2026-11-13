// The current step of the agents a page shows working (live cards, I5): their newest events once, then each event of
// their `session/<id>` topic from the worker's stream. One subscription per page, replaced when the set changes.

import type { EventBody } from "@/gen/EventBody";
import { request, subscribe, type BusEvent } from "../../shared/messages";

const steps = new Map<string, string>();
let watched = "";
let unsubscribe: (() => void) | null = null;
let changed: () => void = () => undefined;

function stepOf(body: EventBody): string | null {
  const b = body.kind === "subagent_event" ? body.body : body;
  switch (b.kind) {
    case "tool_call": return `${b.name} ${b.summary}`.trim().slice(0, 120);
    case "reasoning": return "Thinking";
    case "assistant_delta": case "assistant_message": return "Writing";
    case "file_change": return `Editing ${b.path}`;
    case "background_task": return b.status === "running" ? `Background: ${b.description}`.slice(0, 120) : null;
    case "ask_user": return "Asks you a question";
    case "turn_end": return "";
    default: return null;
  }
}

/** Watches these sessions; `onChange` re-renders the cards. */
export function watch(ids: string[], onChange: () => void) {
  changed = onChange;
  const key = [...new Set(ids)].sort().join(",");
  if (key === watched) return;
  watched = key;
  unsubscribe?.();
  unsubscribe = null;
  if (!key) return;
  const list = key.split(",");
  for (const id of list) {
    if (steps.has(id)) continue;
    steps.set(id, "");
    void request("session.events", { id, tail: 30 }).then((page) => {
      let step = "";
      for (const e of page.events) { const s = stepOf(e.body); if (s !== null) step = s; }
      if (!steps.get(id)) { steps.set(id, step); changed(); }
    }).catch(() => undefined);
  }
  unsubscribe = subscribe(list.map((id) => `session/${id}`), {
    event: (e: BusEvent) => {
      const id = e.topic.slice("session/".length);
      const s = stepOf((e.payload as { body: EventBody }).body);
      if (s === null || steps.get(id) === s) return;
      steps.set(id, s);
      changed();
    },
  });
}

export const stepOfSession = (id: string) => steps.get(id) || null;
