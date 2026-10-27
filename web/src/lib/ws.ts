// One WebSocket to the daemon. Events update the TanStack Query cache and the session logs directly.
//
// Unreliable networks: every event carries a delivery number (`bus`). After a reconnect the window asks
// for the events after the last one it received (`?boot=&since=`); the daemon replays them and sends
// `replayed`, or sends `resync` when it can't, and the window then reloads everything it shows. The
// daemon pings every 25 s; a socket silent for 60 s is replaced. Returning to the page, coming back
// online or restoring it from the back/forward cache checks the socket at once instead of waiting.

import { daemonSocket, isExtension } from "./connection.ts";
import { api } from "./api";
import { rememberSuccessfulSession, useComposer } from "@/stores/composer";
import type { QueryClient } from "@tanstack/react-query";
import type { Session } from "@/gen/Session";
import type { SessionEvent } from "@/gen/SessionEvent";
import { idleMs, isUserActive, onUserInput } from "./activity";
import { reloadSessionLogs, useSessionLogs } from "./sessionStore";

export type BusEvent = { bus: number; seq: number; topic: string; type: string; payload: unknown };
type Listener = (ev: BusEvent) => void;

const listeners = new Set<Listener>();
let connected = false;
const connectionListeners = new Set<(c: boolean) => void>();
let socket: WebSocket | null = null;
/** When this window last received anything from the daemon (epoch ms), null before the first message. */
let lastMessage: number | null = null;

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
  if (connected === c) return;
  connected = c;
  connectionListeners.forEach((fn) => fn(c));
}

/**
 * When the last message from the daemon arrived (epoch ms; null before the first one). It does not
 * change while disconnected, so after `onConnectionChange` reports false it is the "offline since" time.
 */
export function lastMessageAt(): number | null {
  return lastMessage;
}

let presence: { focused: boolean; sessions: string[] } = { focused: false, sessions: [] };
let presenceSentAt = 0;
let sentActive = false;

function writePresence() {
  if (socket?.readyState !== WebSocket.OPEN) return;
  presenceSentAt = Date.now();
  sentActive = isUserActive();
  socket.send(JSON.stringify({ type: "presence", ...presence, idle_ms: idleMs() }));
}

/** Tells the daemon which sessions are on screen, so it does not notify about them while someone uses this window. */
export function sendPresence(focused: boolean, sessions: string[]) {
  presence = { focused, sessions };
  writePresence();
}

// The daemon requires input in the last minute: refresh the last input time while a focused window shows a
// session, at once after an idle report, otherwise at most every 15 s.
onUserInput(() => {
  if (presence.focused && presence.sessions.length && (!sentActive || Date.now() - presenceSentAt > 15_000)) writePresence();
});

/** No message for this long (the daemon pings every 25 s): the connection is dead, replace it. */
const STALE_MS = 60_000;
/** On wake-up, a socket quiet for this long must answer a ping within PROBE_WAIT_MS. */
const PROBE_AFTER_MS = 30_000;
const PROBE_WAIT_MS = 5_000;
/** A connection attempt that takes longer than this on wake-up is started again. */
const CONNECT_WAIT_MS = 5_000;

export function connectEvents(queryClient: QueryClient) {
  let retry = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let build: string | null = null;
  // The daemon run and the last delivery number this window has applied. Both change together, once
  // the daemon confirmed replay or resync, so a reconnect never skips events.
  let position: { boot: string; bus: number } | null = null;
  let offered: { boot: string; bus: number } | null = null;
  let openedAt = 0;
  let heardAt = 0;

  const reloadAll = () => {
    void queryClient.invalidateQueries();
    reloadSessionLogs();
  };

  const handle = (ev: BusEvent & { build?: string; boot?: string }) => {
    if (ev.type === "ping" || ev.type === "pong") return;
    if (ev.type === "hello") {
      // A new UI build was installed (daemon restarted): reload to run it.
      if (build && ev.build !== build) location.reload();
      build = ev.build ?? null;
      offered = { boot: ev.boot ?? "", bus: ev.bus };
      return;
    }
    if (ev.type === "replayed" || ev.type === "resync") {
      window.dispatchEvent(new Event("launchpad-reconnected"));
      if (offered) position = offered;
      offered = null;
      // Missed events were replayed in order: nothing else to reload.
      if (ev.type === "resync") reloadAll();
      else reloadSessionLogs(true);
      return;
    }
    // Only events (never control messages) advance the position.
    if (position) position = { ...position, bus: ev.bus };
    const topic: string = ev.topic;
    if (topic === "status") {
      queryClient.setQueryData(["status"], ev.payload);
    } else if (topic === "inbox") {
      if (!queryClient.isMutating({ mutationKey: ["inbox-action"] })) void queryClient.invalidateQueries({ queryKey: ["inbox"] });
      void queryClient.invalidateQueries({ queryKey: ["inbox-preview"] });
    } else if (topic === "config") {
      void queryClient.invalidateQueries({ queryKey: ["config"] });
    } else if (topic === "proposals") {
      void queryClient.invalidateQueries({ queryKey: ["proposals"] });
    } else if (topic === "workspaces") {
      void queryClient.invalidateQueries({ queryKey: ["workspaces"] });
    } else if (topic === "projects") {
      void queryClient.invalidateQueries({ queryKey: ["projects"] });
    } else if (topic === "predrafts") {
      void queryClient.invalidateQueries({ queryKey: ["predrafts"] });
    } else if (topic === "worktrees") {
      void queryClient.invalidateQueries({ queryKey: ["worktrees"] });
      void queryClient.invalidateQueries({ queryKey: ["code-sources"] });
    } else if (topic === "suggestions") {
      void queryClient.invalidateQueries({ queryKey: ["suggestions"] });
    } else if (topic === "terminals") {
      void queryClient.invalidateQueries({ queryKey: ["terminals"] });
    } else if (topic === "crons") {
      void queryClient.invalidateQueries({ queryKey: ["crons"] });
    } else if (topic === "drafts") {
      useComposer.getState().remoteChanged(ev.payload as { workspace: string | null; revision: number; client: string | null });
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

  let opening = false;
  let generation = 0;
  const open = async (): Promise<void> => {
    if (opening) return;
    opening = true;
    const attempt = ++generation;
    try {
      clearTimeout(timer);
      timer = undefined;
      const query = position ? `?boot=${encodeURIComponent(position.boot)}&since=${position.bus}` : "";
      const ticket = isExtension ? await api.post<{ ticket: string }>("/api/devices/socket-ticket") : null;
      if (attempt !== generation) return;
      const ws = new WebSocket(daemonSocket(`/ws${query}${ticket ? `${query ? "&" : "?"}ticket=${encodeURIComponent(ticket.ticket)}` : ""}`));
      socket = ws;
      offered = null;
      openedAt = heardAt = Date.now();
      ws.onopen = () => {
        if (socket !== ws) return;
        retry = 0;
        heardAt = Date.now();
        setConnected(true);
        writePresence();
      };
      ws.onmessage = (msg) => {
        if (socket !== ws) return;
        heardAt = lastMessage = Date.now();
        handle(JSON.parse(msg.data));
      };
      ws.onclose = () => {
        if (socket !== ws) return;
        socket = null;
        // Revocation closes the socket; an authenticated probe triggers storage cleanup on 401.
        void api.get("/api/status").catch(() => {});
        setConnected(false);
        retry = Math.min(retry + 1, 10);
        timer = setTimeout(open, Math.min(250 * 2 ** retry, 5000));
      };
    } catch {
      setConnected(false);
      timer = setTimeout(open, 3000);
    } finally {
      opening = false;
      if (attempt !== generation) void open();
    }
  };

  /** Drops the current socket without waiting for its close handshake (a dead network may never finish it). */
  const restart = () => {
    generation++;
    const old = socket;
    socket = null;
    if (old) {
      old.onopen = old.onmessage = old.onclose = null;
      old.close();
    }
    setConnected(false);
    retry = 0;
    open();
  };

  const probe = () => {
    const ws = socket;
    if (ws?.readyState !== WebSocket.OPEN) return;
    const sentAt = Date.now();
    ws.send(JSON.stringify({ type: "ping" }));
    setTimeout(() => { if (socket === ws && heardAt < sentAt) restart(); }, PROBE_WAIT_MS);
  };

  /** Checks the connection now. `suspect`: the network changed, so even a recently active socket may be dead. */
  const check = (suspect: boolean) => {
    const ws = socket;
    if (!ws) { retry = 0; open(); return; }
    if (ws.readyState === WebSocket.CONNECTING) { if (Date.now() - openedAt > CONNECT_WAIT_MS) restart(); return; }
    if (ws.readyState !== WebSocket.OPEN) { restart(); return; }
    const quiet = Date.now() - heardAt;
    if (quiet > STALE_MS) restart();
    else if (suspect || quiet > PROBE_AFTER_MS) probe();
  };

  // Background tabs throttle timers; the visibility check covers the return.
  setInterval(() => {
    const ws = socket;
    if (ws?.readyState === WebSocket.OPEN && Date.now() - heardAt > STALE_MS) restart();
  }, 5_000);
  document.addEventListener("visibilitychange", () => { if (document.visibilityState === "visible") check(false); });
  window.addEventListener("pageshow", (e) => { if (e.persisted) check(true); });
  window.addEventListener("online", () => check(true));
  open();
}
