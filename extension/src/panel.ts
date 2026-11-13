// Pair before loading shared stores and their background writers.
import "./styles.css";
import { applyTextSize } from "./textSize";

applyTextSize();

const root = document.getElementById("root")!;
document.documentElement.classList.add("x-panel");
if (matchMedia("(prefers-color-scheme: dark)").matches) document.documentElement.dataset.theme = "dark";
const logo = chrome.runtime.getURL("icons/128.png");
root.innerHTML = `<div class="x-app"><div class="x-center">
  <img src="${logo}" width="56" height="56" alt="" style="border-radius:16px">
  <h1>Connect to Launchpad</h1>
  <p class="x-muted">Connect this panel to your Launchpad Home with a pairing code.</p>
  <form id="remote" style="display:grid;gap:8px;width:100%;text-align:left"><label class="x-small x-faint">Launchpad address<input id="origin" class="x-field" style="margin-top:4px" type="url" required placeholder="https://your-machine.ts.net"></label><button class="x-btn x-pri" type="submit">Show pairing code</button></form>
  <p id="lead" class="x-muted" hidden>Open Launchpad on a paired device → Settings → Devices, and approve this code:</p>
  <p id="code" class="x-code" hidden aria-live="polite"></p>
  <p id="status" class="x-small x-muted" role="status"></p>
</div></div>`;
const status = document.getElementById("status")!;
let polling = 0;

async function json(origin: string, path: string, body?: unknown) {
  const response = await fetch(origin + path, { credentials: "include", method: body === undefined ? "GET" : "POST", headers: body === undefined ? {} : { "Content-Type": "application/json" }, body: body === undefined ? undefined : JSON.stringify(body) });
  const data = await response.json();
  if (!response.ok) throw new Error(data.error ?? response.statusText);
  return data;
}
function originUrl(input: string) {
  const url = new URL(input);
  if (url.username || url.password || url.search || url.hash || url.pathname !== "/" || !(url.protocol === "https:" || url.protocol === "http:" && ["127.0.0.1", "localhost"].includes(url.hostname))) throw new Error("Use an HTTPS Launchpad origin, or HTTP on localhost.");
  return url.origin;
}
async function ready(origin: string) {
  const devices = await json(origin, "/api/devices"); // Verify cookies actually work before loading authenticated stores.
  const current = devices.find((d: { current: boolean }) => d.current);
  if (!current) throw new Error("Pair this extension again");
  localStorage.setItem("launchpad-extension-device", current.id);
  await chrome.storage.local.set({ daemon: origin });
  localStorage.setItem("launchpad-daemon-origin", origin);
  const app = await import("./app");
  app.mount();
}
document.getElementById("remote")!.onsubmit = (event) => {
  event.preventDefault();
  const attempt = ++polling;
  void (async () => {
    const origin = originUrl((document.getElementById("origin") as HTMLInputElement).value.trim());
    if (!await chrome.permissions.request({ origins: [`${origin}/*`] })) throw new Error("Launchpad connection permission was declined");
    const pair = await json(origin, "/auth/reverse/start", { name: "Chrome extension" });
    const code = document.getElementById("code")!;
    code.textContent = `${pair.code.slice(0, 4)}-${pair.code.slice(4)}`;
    code.hidden = false;
    document.getElementById("lead")!.hidden = false;
    status.textContent = "Waiting for approval… keep this panel open.";
    const poll = async () => {
      if (attempt !== polling) return;
      if (Date.now() >= pair.expires_at * 1000) { status.textContent = "Pairing code expired. Show a new code."; return; }
      try {
        const result = await json(origin, "/auth/reverse/poll", { code: pair.code, secret: pair.secret });
        if (result.ok) { await ready(origin); return; }
      } catch (e) { status.textContent = `Pairing: ${String(e)}`; }
      setTimeout(() => void poll(), 2500);
    };
    setTimeout(() => void poll(), 2100);
  })().catch((e) => { status.textContent = String(e); });
};
void chrome.storage.local.get("daemon").then(async ({ daemon }) => {
  if (daemon) { try { await ready(originUrl(String(daemon))); } catch { status.textContent = "Can\u2019t reach Launchpad, or this browser was signed out. Connect again when Launchpad is running."; } }
});
