import { readPage } from "./page";

type Page = { tab: number; document: string; url: string; title: string };
let socket: WebSocket | null = null;
let connecting: Promise<void> | null = null;
let daemon = "";
let device = "";
let position: { boot: string; bus: number } | null = null;
let offered: { boot: string; bus: number } | null = null;
let lastMessage = 0;

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
      if (ev.topic === "browser" && ev.type === "read" && ev.payload.device === device) {
        const req = ev.payload;
        void (async () => {
          let result;
          try {
            const page = req.page as Page;
            const [injection] = await chrome.scripting.executeScript({ target: { tabId: page.tab, frameIds: [0] }, func: readPage, args: [{ document: page.document, url: page.url }, req.tool, req.args] });
            if (!injection?.result) throw new Error("Page is unavailable; add it again");
            result = injection.result;
          } catch (error) { result = { error: String(error) }; }
          await api(`/api/browser/replies/${req.id}`, result);
        })().catch(() => { /* A disconnected read expires in the broker; it never performs input. */ });
      }
    };
    ws.onclose = () => { if (socket === ws) { socket = null; setTimeout(() => void connect().catch(() => {}), 2000); } };
  })().finally(() => { connecting = null; });
  return connecting;
}

export async function capture(tabId: number): Promise<Page> {
  const [result] = await chrome.scripting.executeScript({ target: { tabId, frameIds: [0] }, func: readPage });
  if (!result?.result) throw new Error("Chrome cannot read this page. Open an ordinary HTTP or HTTPS page.");
  return { tab: tabId, ...result.result };
}

chrome.runtime.onInstalled.addListener(() => {
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
  if (message.type === "capture") { void capture(message.tab).then(respond, (e) => respond({ error: String(e) })); return true; }
});
chrome.alarms.onAlarm.addListener(() => void connect().catch(() => {}));
chrome.runtime.onStartup.addListener(() => void connect().catch(() => {}));
void connect().catch(() => {});
