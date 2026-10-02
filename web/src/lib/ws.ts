// One WebSocket to the daemon. Events update the TanStack Query cache directly.

import type { QueryClient } from "@tanstack/react-query";

export type BusEvent = { seq: number; topic: string; type: string; payload: unknown };
type Listener = (ev: BusEvent) => void;

const listeners = new Set<Listener>();
let connected = false;
const connectionListeners = new Set<(c: boolean) => void>();

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

export function connectEvents(queryClient: QueryClient) {
  let retry = 0;
  let build: string | null = null;
  const open = () => {
    const ws = new WebSocket(`${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/ws`);
    ws.onopen = () => {
      retry = 0;
      setConnected(true);
      // Anything may have changed while disconnected.
      void queryClient.invalidateQueries();
    };
    ws.onmessage = (msg) => {
      const ev = JSON.parse(msg.data);
      if (ev.type === "resync") {
        void queryClient.invalidateQueries();
        return;
      }
      if (ev.type === "hello") {
        // A new UI build was installed (daemon restarted): reload to run it.
        if (build && ev.build !== build) location.reload();
        build = ev.build;
        return;
      }
      if (ev.topic === "status") {
        queryClient.setQueryData(["status"], ev.payload);
      } else if (ev.topic === "config") {
        void queryClient.invalidateQueries({ queryKey: ["config"] });
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
