// Subscribed clients (side panels and content scripts, through `subscribe` of shared/messages.ts): the worker pushes
// state changes to all of them and each bus event to those that asked for its topic.

import { PORT, type BusEvent, type ConnectionState, type Push, type Subscribe } from "../shared/messages";

const clients = new Map<chrome.runtime.Port, string[] | "*">();

export function listen(current: () => ConnectionState, ready: Promise<unknown>) {
  chrome.runtime.onConnect.addListener((port) => {
    if (port.name !== PORT || port.sender?.id !== chrome.runtime.id) return;
    port.onMessage.addListener((m: Subscribe) => {
      if (m?.type !== "subscribe") return;
      clients.set(port, m.topics === "*" ? "*" : m.topics.filter((t) => typeof t === "string"));
      void ready.then(() => post(port, { type: "state", state: current() }));
    });
    port.onDisconnect.addListener(() => clients.delete(port));
  });
}

function post(port: chrome.runtime.Port, message: Push) {
  try { port.postMessage(message); } catch { clients.delete(port); }
}

export function pushState(state: ConnectionState) {
  for (const port of clients.keys()) post(port, { type: "state", state });
}

export function pushEvent(event: BusEvent) {
  for (const [port, topics] of clients) if (topics === "*" || topics.includes(event.topic)) post(port, { type: "event", event });
}

export function pushResync() {
  for (const port of clients.keys()) post(port, { type: "resync" });
}
