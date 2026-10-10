// One WebSocket to the daemon. Events update the TanStack Query cache and the session logs directly.

import { rememberSuccessfulSession } from "@/stores/composer";
import type { QueryClient } from "@tanstack/react-query";
import type { Session } from "@/gen/Session";
import type { SessionEvent } from "@/gen/SessionEvent";
import { reloadSessionLogs, useSessionLogs } from "./sessionStore";

export type BusEvent = { seq: number; topic: string; type: string; payload: unknown };
type Listener = (ev: BusEvent) => void;

const listeners = new Set<Listener>();
let connected = false;
const connectionListeners = new Set<(c: boolean) => void>();
let socket: WebSocket | null = null;

/** Subscribes to every bus event. Returns the unsubscribe function. */
export function onEvent(fn: Listener): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

export function onConnectionChange(fn: (c: boolean) => void): () => void {
  connectionListeners.add(fn);
  fn(connected);
  return () => connectionListeners.delete(fn);
}

function setConnected(c: boolean) {
  connected = c;
  connectionListeners.forEach((fn) => fn(c));
}

let presence: { focused: boolean; sessions: string[] } = { focused: false, sessions: [] };

/** Tells the daemon which sessions are on screen, so it does not notify about them. */
export function sendPresence(focused: boolean, sessions: string[]) {
  presence = { focused, sessions };
  if (socket?.readyState === WebSocket.OPEN) socket.send(JSON.stringify({ type: "presence", ...presence }));
}

export function connectEvents(queryClient: QueryClient) {
  let retry = 0;
  let build: string | null = null;
  const open = () => {
    const ws = new WebSocket(`${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/ws`);
    socket = ws;
    ws.onopen = () => {
      retry = 0;
      setConnected(true);
      ws.send(JSON.stringify({ type: "presence", ...presence }));
      // Anything may have changed while disconnected.
      void queryClient.invalidateQueries();
      reloadSessionLogs();
    };
    ws.onmessage = (msg) => {
      const ev = JSON.parse(msg.data);
      if (ev.type === "resync") {
        void queryClient.invalidateQueries();
        reloadSessionLogs();
        return;
      }
      if (ev.type === "hello") {
        // A new UI build was installed (daemon restarted): reload to run it.
        if (build && ev.build !== build) location.reload();
        build = ev.build;
        return;
      }
      const topic: string = ev.topic;
      if (topic === "status") {
        queryClient.setQueryData(["status"], ev.payload);
      } else if (topic === "inbox") {
        if (!queryClient.isMutating({ mutationKey: ["inbox-action"] })) void queryClient.invalidateQueries({ queryKey: ["inbox"] });
        void queryClient.invalidateQueries({ queryKey: ["inbox-preview"] });
      } else if (topic === "config") {
        void queryClient.invalidateQueries({ queryKey: ["config"] });
      } else if (topic === "workspaces") {
        void queryClient.invalidateQueries({ queryKey: ["workspaces"] });
      } else if (topic === "projects") {
        void queryClient.invalidateQueries({ queryKey: ["projects"] });
      } else if (topic === "predrafts") {
        void queryClient.invalidateQueries({ queryKey: ["predrafts"] });
        void queryClient.invalidateQueries({ queryKey: ["code-sources"] });
      } else if (topic === "suggestions") {
        void queryClient.invalidateQueries({ queryKey: ["suggestions"] });
      } else if (topic === "terminals") {
        void queryClient.invalidateQueries({ queryKey: ["terminals"] });
      } else if (topic === "crons") {
        void queryClient.invalidateQueries({ queryKey: ["crons"] });
      } else if (topic === "schedules") {
        void queryClient.invalidateQueries({ queryKey: ["schedules"] });
      } else if (topic === "sessions") {
        if (ev.type === "updated") {
          const s = ev.payload as Session;
          const previous = queryClient.getQueryData<Session[]>(["sessions"])?.find((p) => p.id === s.id);
          if (previous?.status !== s.status) void queryClient.invalidateQueries({ queryKey: ["predrafts"] });
          rememberSuccessfulSession(s);
          queryClient.setQueryData<Session[]>(["sessions"], (old) => {
            if (!old) return old;
            const i = old.findIndex((o) => o.id === s.id);
            if (i < 0) return [s, ...old];
            const next = [...old];
            next[i] = s;
            return next;
          });
        } else {
          void queryClient.invalidateQueries({ queryKey: ["sessions"] });
        }
      } else if (topic.startsWith("session/")) {
        useSessionLogs.getState().receive(ev.payload as SessionEvent);
      }
      listeners.forEach((fn) => fn(ev as BusEvent));
    };
    ws.onclose = () => {
      setConnected(false);
      retry = Math.min(retry + 1, 10);
      setTimeout(open, Math.min(250 * 2 ** retry, 5000));
    };
  };
  open();
}
