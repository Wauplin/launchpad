// Events of the sessions shown on screen. Stored events come from the API and the WebSocket in
// sequence order; a gap triggers a fetch of what is missing. Live-only events (text deltas, command
// output) are kept apart and dropped when the final event arrives.

import { create } from "zustand";
import type { SessionEvent } from "@/gen/SessionEvent";
import { api } from "./api";

export type SessionLog = {
  events: SessionEvent[];
  lastSeq: number;
  loaded: boolean;
  /** item_id → streamed assistant text not final yet. */
  deltas: Record<string, string>;
  /** item_id → live command output. */
  outputs: Record<string, string>;
};

const empty = (): SessionLog => ({ events: [], lastSeq: 0, loaded: false, deltas: {}, outputs: {} });

type Store = {
  logs: Record<string, SessionLog>;
  load: (id: string) => Promise<void>;
  receive: (ev: SessionEvent) => void;
};

const loading = new Set<string>();

export const useSessionLogs = create<Store>((set, get) => ({
  logs: {},
  load: async (id) => {
    if (loading.has(id)) return;
    loading.add(id);
    try {
      const after = get().logs[id]?.lastSeq ?? 0;
      const evs = await api.get<SessionEvent[]>(`/api/sessions/${id}/events?after=${after}`);
      set((s) => {
        const log = s.logs[id] ?? empty();
        const fresh = evs.filter((e) => e.seq > log.lastSeq);
        const events = [...log.events, ...fresh];
        const lastSeq = events.length ? events[events.length - 1].seq : log.lastSeq;
        return { logs: { ...s.logs, [id]: settle({ ...log, events, lastSeq, loaded: true }, fresh) } };
      });
    } finally {
      loading.delete(id);
    }
  },
  receive: (ev) => {
    const id = ev.session_id;
    const log = get().logs[id];
    if (!log) return; // not on screen
    if (ev.seq === 0) {
      const wrapped = ev.body;
      const b = wrapped.kind === "subagent_event" ? wrapped.body : wrapped;
      const prefix = wrapped.kind === "subagent_event" ? `${wrapped.agent_id}\0` : "";
      if (b.kind === "assistant_delta") {
        set((s) => ({ logs: { ...s.logs, [id]: { ...log, deltas: { ...log.deltas, [prefix + b.item_id]: (log.deltas[prefix + b.item_id] ?? "") + b.text } } } }));
      } else if (b.kind === "tool_output") {
        const text = ((log.outputs[prefix + b.item_id] ?? "") + b.text).slice(-8000);
        set((s) => ({ logs: { ...s.logs, [id]: { ...log, outputs: { ...log.outputs, [prefix + b.item_id]: text } } } }));
      }
      return;
    }
    if (ev.seq <= log.lastSeq) return;
    if (ev.seq !== log.lastSeq + 1) {
      void get().load(id);
      return;
    }
    set((s) => ({ logs: { ...s.logs, [id]: settle({ ...log, events: [...log.events, ev], lastSeq: ev.seq }, [ev]) } }));
  },
}));

/** Drops live text and output that a final event replaces. */
function settle(log: SessionLog, fresh: SessionEvent[]): SessionLog {
  let { deltas, outputs } = log;
  for (const e of fresh) {
    const wrapped = e.body;
    const b = wrapped.kind === "subagent_event" ? wrapped.body : wrapped;
    const prefix = wrapped.kind === "subagent_event" ? `${wrapped.agent_id}\0` : "";
    if (b.kind === "assistant_message" && prefix + b.item_id in deltas) {
      deltas = { ...deltas };
      delete deltas[prefix + b.item_id];
    } else if (b.kind === "tool_result" && prefix + b.item_id in outputs) {
      outputs = { ...outputs };
      delete outputs[prefix + b.item_id];
    } else if (b.kind === "turn_end") {
      deltas = Object.fromEntries(Object.entries(deltas).filter(([key]) => prefix ? !key.startsWith(prefix) : key.includes("\0")));
    }
  }
  return { ...log, deltas, outputs };
}

/** Reloads every session on screen (after a reconnect). */
export function reloadSessionLogs() {
  const { logs, load } = useSessionLogs.getState();
  Object.keys(logs).forEach((id) => void load(id));
}
