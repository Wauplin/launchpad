// Events of the sessions shown on screen. Stored events come from the API and the WebSocket in
// sequence order; a gap triggers a fetch of what is missing. Live-only events (text deltas, command
// output) are kept apart and dropped when the final event arrives. A session opens with its newest
// events (tail-first); earlier pages load on request, and `facts` summarizes what is not loaded yet.

import { create } from "zustand";
import type { EarlierFacts } from "@/gen/EarlierFacts";
import type { Session } from "@/gen/Session";
import type { SessionEvent } from "@/gen/SessionEvent";
import type { SessionEventPage } from "@/gen/SessionEventPage";
import { api } from "./api.ts";

export type SessionLog = {
  events: SessionEvent[];
  lastSeq: number;
  loaded: boolean;
  loading: boolean;
  error: string | null;
  /** item_id → streamed assistant text not final yet. */
  deltas: Record<string, string>;
  /** item_id → live command output. */
  outputs: Record<string, string>;
  /** Facts about the stored events before `events[0]`; null once the log is loaded from its start. */
  earlier: EarlierFacts | null;
};

export const emptyLog = (): SessionLog => ({ events: [], lastSeq: 0, loaded: false, loading: false, error: null, deltas: {}, outputs: {}, earlier: null });
const empty = emptyLog;

/** Events per page: the first page is the newest ones, then pages backward. */
const PAGE = 300;

type Store = {
  logs: Record<string, SessionLog>;
  load: (id: string) => Promise<void>;
  /** Prepends the page of events before the first loaded one. */
  loadEarlier: (id: string) => Promise<void>;
  receive: (ev: SessionEvent) => void;
};

const loading = new Set<string>();
// A gap or reconnect during a fetch needs another pass after that response is applied.
const dirty = new Set<string>();

export const useSessionLogs = create<Store>((set, get) => ({
  logs: {},
  load: async (id) => {
    if (loading.has(id)) { dirty.add(id); return; }
    loading.add(id);
    set((s) => ({ logs: { ...s.logs, [id]: { ...(s.logs[id] ?? empty()), loading: true, error: null } } }));
    try {
      do {
        dirty.delete(id);
        const current = get().logs[id];
        if (!current?.loaded) {
          const page = await api.get<SessionEventPage>(`/api/sessions/${id}/events/page?tail=${PAGE}`);
          set((s) => {
            const log = s.logs[id] ?? empty();
            // Live events received meanwhile are newer than the page or already in it.
            const events = [...page.events, ...log.events.filter((e) => e.seq > (page.events.at(-1)?.seq ?? 0))];
            const lastSeq = events.length ? events[events.length - 1].seq : 0;
            return { logs: { ...s.logs, [id]: settle({ ...log, events, lastSeq, loaded: true, earlier: page.facts }, events) } };
          });
        } else {
          const after = current.lastSeq;
          const evs = await api.get<SessionEvent[]>(`/api/sessions/${id}/events?after=${after}`);
          set((s) => {
            const log = s.logs[id] ?? empty();
            const fresh = evs.filter((e) => e.seq > log.lastSeq);
            const events = [...log.events, ...fresh];
            const lastSeq = events.length ? events[events.length - 1].seq : log.lastSeq;
            return { logs: { ...s.logs, [id]: settle({ ...log, events, lastSeq, loaded: true }, fresh) } };
          });
        }
      } while (dirty.has(id));
    } catch (error) {
      set((s) => ({ logs: { ...s.logs, [id]: { ...s.logs[id], error: error instanceof Error ? error.message : "Could not load session events." } } }));
    } finally {
      loading.delete(id);
      dirty.delete(id);
      set((s) => ({ logs: { ...s.logs, [id]: { ...s.logs[id], loading: false } } }));
    }
  },
  loadEarlier: async (id) => {
    const first = get().logs[id]?.events[0]?.seq;
    if (first === undefined || !get().logs[id]?.earlier || loading.has(`${id}<`)) return;
    loading.add(`${id}<`);
    try {
      const page = await api.get<SessionEventPage>(`/api/sessions/${id}/events/page?before=${first}&limit=${PAGE}`);
      set((s) => {
        const log = s.logs[id];
        if (!log || log.events[0]?.seq !== first) return s;
        return { logs: { ...s.logs, [id]: { ...log, events: [...page.events, ...log.events], earlier: page.facts } } };
      });
    } finally {
      loading.delete(`${id}<`);
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
export function reloadSessionLogs(onlyIncomplete = false) {
  const { logs, load } = useSessionLogs.getState();
  Object.keys(logs).filter((id) => !onlyIncomplete || !logs[id].loaded || logs[id].error).forEach((id) => void load(id));
}

/** Bucket imports announce their cursor; only viewers with this trace loaded fetch its new events. */
export function catchUpSyncedTrace(s: Pick<Session, "id" | "imported_from" | "last_seq">): Promise<void> | undefined {
  const { logs, load } = useSessionLogs.getState();
  if (s.imported_from && logs[s.id] && logs[s.id].lastSeq < s.last_seq) return load(s.id);
}
