// Home's bus as a fetch stream (`GET /api/events`, server-sent events): WebSockets cannot carry the Hugging Face token.
// The stream resumes from the last event it applied (`Last-Event-ID: <boot>:<bus>`, kept in storage), reconnects with
// backoff, and keeps the worker alive: each chunk calls an extension API (which resets the worker's idle timer), and the
// `heartbeat` alarm restarts the stream when Chrome stopped the worker anyway.

import type { BusEvent } from "../shared/messages";
import { contextEvent, contextResync } from "./context";
import { current, homeFetch, loadToday, loadWorkspaces, todayChanged, update } from "./home";
import { homeStatus } from "./hf";
import { pushEvent, pushResync } from "./ports";
import { local, setLocal } from "./store";

export const HEARTBEAT_ALARM = "heartbeat";
/** Home sends a keepalive comment every 15 s: a stream silent for this long is dead. */
const SILENT_MS = 45_000;
const BACKOFF_MS = [1_000, 2_000, 5_000, 10_000, 20_000, 30_000];

let running = false;
let controller: AbortController | null = null;
let generation = 0;

export function streaming() {
  return running;
}

export function startEvents() {
  if (running) return;
  running = true;
  void chrome.alarms.create(HEARTBEAT_ALARM, { periodInMinutes: 0.5 });
  const mine = ++generation;
  void loop(mine);
}

export function stopEvents() {
  running = false;
  generation++;
  controller?.abort();
  controller = null;
  void chrome.alarms.clear(HEARTBEAT_ALARM);
}

async function loop(mine: number) {
  let attempt = 0;
  while (mine === generation) {
    const opened = await once(mine).catch((e) => { if (mine === generation) console.warn("event stream", e); return false; });
    if (mine !== generation || current().phase === "revoked") break;
    if (opened) attempt = 0;
    await offline();
    await new Promise((r) => setTimeout(r, BACKOFF_MS[Math.min(attempt++, BACKOFF_MS.length - 1)]));
  }
  if (mine === generation) running = false;
}

/** Why the stream failed: a Space that wakes up says so through `/_home/status`. */
async function offline() {
  const home = current().home;
  if (!home || current().phase === "revoked") return;
  const status = home.kind === "space" ? await homeStatus(home.origin) : null;
  update({ phase: status?.kind === "starting" ? "starting" : "offline" });
}

let keepAliveAt = 0;
function keepWorkerAlive() {
  if (Date.now() - keepAliveAt < 20_000) return;
  keepAliveAt = Date.now();
  void chrome.runtime.getPlatformInfo();
}

/** One stream. Resolves true when it opened (then ended), throws when it could not open. */
async function once(mine: number): Promise<boolean> {
  const position = await local("events.position");
  controller = new AbortController();
  const abort = controller;
  const response = await homeFetch("/api/events", { signal: abort.signal, headers: { Accept: "text/event-stream", ...(position ? { "Last-Event-ID": `${position.boot}:${position.bus}` } : {}) } });
  if (!response.ok || !response.body) throw new Error(`Home answered ${response.status}`);
  let silent = setTimeout(() => abort.abort(), SILENT_MS);
  let offered: { boot: string; bus: number } | null = null;
  let at = position;
  let savedAt = 0;
  const save = async (force = false) => {
    if (!at || (!force && Date.now() - savedAt < 2_000)) return;
    savedAt = Date.now();
    await setLocal({ "events.position": at });
  };
  const handle = async (data: string) => {
    let ev: BusEvent & { boot?: string; build?: string };
    try { ev = JSON.parse(data); } catch { return; }
    if (ev.type === "hello") { offered = { boot: ev.boot ?? "", bus: ev.bus }; return; }
    if (ev.type === "replayed" || ev.type === "resync") {
      if (offered) at = offered;
      offered = null;
      await save(true);
      update({ phase: "online", error: null });
      // Every (re)opened stream reloads the workspaces and the count: cheap, and the worker may have just started.
      await loadWorkspaces().then(loadToday).catch((e) => console.warn("reload", e));
      if (ev.type === "resync") { contextResync(); pushResync(); }
      return;
    }
    if (at) at = { ...at, bus: ev.bus };
    void save();
    if (ev.topic === "today") todayChanged();
    else if (ev.topic === "workspaces") void loadWorkspaces().then(() => todayChanged()).catch((e) => console.warn("workspaces", e));
    contextEvent(ev);
    pushEvent(ev);
  };
  const reader = response.body.pipeThrough(new TextDecoderStream()).getReader();
  let buffer = "";
  try {
    for (;;) {
      const { value, done } = await reader.read();
      if (done || mine !== generation) break;
      clearTimeout(silent);
      silent = setTimeout(() => abort.abort(), SILENT_MS);
      keepWorkerAlive();
      buffer += value.replace(/\r\n?/g, "\n");
      let end: number;
      while ((end = buffer.indexOf("\n\n")) >= 0) {
        const block = buffer.slice(0, end);
        buffer = buffer.slice(end + 2);
        const data = block.split("\n").filter((l) => l.startsWith("data:")).map((l) => l.slice(5).replace(/^ /, "")).join("\n");
        if (data) await handle(data);
      }
    }
  } catch (e) {
    if (!abort.signal.aborted) throw e;
  } finally {
    clearTimeout(silent);
    await save(true).catch(() => undefined);
    reader.releaseLock();
  }
  return true;
}
