import { readPage } from "./page";
import { work, release, releaseAll, reconcile, type Request } from "./work";

type Page = { tab: number; document: string; url: string; title: string };
let socket: WebSocket | null = null;
let connecting: Promise<void> | null = null;
let daemon = "";
let device = "";
let position: { boot: string; bus: number } | null = null;
let offered: { boot: string; bus: number } | null = null;
let lastMessage = 0;
const processing = new Set<string>();

async function api(path: string, body?: unknown) {
  const response = await fetch(`${daemon}${path}`, { credentials: "include", method: body === undefined ? "GET" : "POST", headers: body === undefined ? {} : { "Content-Type": "application/json" }, body: body === undefined ? undefined : JSON.stringify(body) });
  if (!response.ok) throw new Error(`Launchpad: ${response.status} ${await response.text()}`);
  return response.json();
}

async function connect() {
  if (connecting) return connecting;
  connecting = (async () => {
    const stored = await chrome.storage.local.get("daemon");
    if (!stored.daemon) return;
    const origin = String(stored.daemon);
    if (socket && daemon === origin && (socket.readyState === WebSocket.OPEN || socket.readyState === WebSocket.CONNECTING) && Date.now() - lastMessage < 60_000) return;
    await releaseAll();
    socket?.close();
    daemon = origin;
    const devices = await api("/api/devices") as { id: string; current: boolean }[];
    device = devices.find((d) => d.current)?.id ?? "";
    if (!device) throw new Error("Pair this extension again");
    const saved = await chrome.storage.session.get("position") as { position?: { daemon: string; value: { boot: string; bus: number } } };
    if (saved.position?.daemon === daemon) position = saved.position.value;
    const query = position ? `?boot=${encodeURIComponent(position.boot)}&since=${position.bus}` : "";
    const { ticket } = await api("/api/devices/socket-ticket", {});
    const ws = new WebSocket(`${daemon.replace(/^http/, "ws")}/ws${query}${query ? "&" : "?"}ticket=${encodeURIComponent(ticket)}`);
    socket = ws;
    lastMessage = Date.now();
    ws.onmessage = (event) => {
      if (ws !== socket) return;
      lastMessage = Date.now();
      const ev = JSON.parse(event.data);
      if (ev.type === "hello") offered = { boot: ev.boot, bus: ev.bus };
      else if (ev.type === "resync" || ev.type === "replayed") position = offered;
      else if (ev.bus && position) position = { ...position, bus: ev.bus };
      if (position) void chrome.storage.session.set({ position: { daemon, value: position } });
      if (ev.topic === "sessions") void api("/api/sessions").then(reconcile).catch(() => releaseAll());
      if (ev.topic === "browser" && ev.type === "release" && ev.payload.device === device) {
        void release(ev.payload.tab);
        void chrome.runtime.sendMessage({ type: "browser-changed" }).catch(() => {});
      }
      if (ev.topic === "browser" && ev.type === "read" && ev.payload.device === device) {
        const req = ev.payload;
        if (processing.has(req.id)) return;
        processing.add(req.id);
        void (async () => {
          let result;
          try {
            await api(`/api/browser/dispatch/${req.id}`, {});
            // Only the first delivery can dispatch; replay is rejected before prepare/input.
            if (req.phase !== "read") {
              result = await work(req as Request);
              await api(`/api/browser/replies/${req.id}`, result);
              await chrome.storage.session.remove(`action:${req.id}`);
              return;
            }
            let page = req.page as Page;
            if (req.work) {
              const captured = await capture(page.tab);
              if (new URL(captured.url).origin !== new URL(page.url).origin) throw new Error("New site: add the destination explicitly");
              const known = await chrome.storage.session.get(`grant:${req.grant}`);
              if (captured.document !== page.document && !known[`grant:${req.grant}`]) throw new Error("Browser or extension restarted; add the page again");
              await chrome.storage.session.set({ [`grant:${req.grant}`]: true });
              page = captured;
            }
            const [injection] = await chrome.scripting.executeScript({ target: { tabId: page.tab, frameIds: [0] }, func: readPage, args: [{ document: page.document, url: page.url }, req.tool, req.args] });
            if (!injection?.result) throw new Error("Page is unavailable; add it again");
            result = req.work ? { ...injection.result, page } : injection.result;
          } catch (error) { result = { error: String(error) }; }
          await api(`/api/browser/replies/${req.id}`, result);
        })().catch(() => { void release(req.page.tab); }).finally(() => processing.delete(req.id));
      }
    };
    ws.onclose = () => { void releaseAll(); if (socket === ws) { socket = null; setTimeout(() => void connect().catch(() => {}), 2000); } };
  })().finally(() => { connecting = null; });
  return connecting;
}

export async function capture(tabId: number): Promise<Page> {
  const [result] = await chrome.scripting.executeScript({ target: { tabId, frameIds: [0] }, func: readPage });
  if (!result?.result) throw new Error("Chrome cannot read this page. Open an ordinary HTTP or HTTPS page.");
  return { tab: tabId, ...result.result };
}

chrome.runtime.onInstalled.addListener(() => {
  void chrome.storage.session.clear();
  void chrome.sidePanel.setPanelBehavior({ openPanelOnActionClick: true });
  chrome.contextMenus.create({ id: "ask-launchpad", title: "Ask Launchpad about selection", contexts: ["selection"], documentUrlPatterns: ["http://*/*", "https://*/*"] });
  void chrome.alarms.create("reconnect", { periodInMinutes: 1 });
});
chrome.contextMenus.onClicked.addListener((info, tab) => {
  if (info.menuItemId !== "ask-launchpad" || tab?.id == null) return;
  // Open synchronously in the user gesture. Capture is deferred until the panel asks for it.
  void chrome.storage.session.set({ askSelection: { tab: tab.id } });
  void chrome.sidePanel.open({ windowId: tab.windowId });
  void chrome.runtime.sendMessage({ type: "selection-ready" }).catch(() => {});
});
chrome.runtime.onMessage.addListener((message, sender, respond) => {
  if (sender.id !== chrome.runtime.id || !sender.url?.startsWith(chrome.runtime.getURL(""))) return;
  if (message.type === "connect") { void connect().then(() => respond({ ok: true }), (e) => respond({ error: String(e) })); return true; }
  if (message.type === "release") { void release(message.tab).then(() => respond({ ok: true })); return true; }
  if (message.type === "capture") { void capture(message.tab).then(respond, (e) => respond({ error: String(e) })); return true; }
});
chrome.alarms.onAlarm.addListener(() => { void connect().then(() => api("/api/sessions")).then(reconcile).catch(() => releaseAll()); });
setInterval(() => { void api("/api/sessions").then(reconcile).catch(() => releaseAll()); }, 5000);
chrome.runtime.onStartup.addListener(() => { void chrome.storage.session.clear().then(() => connect()).catch(() => {}); });
void connect().catch(() => {});
