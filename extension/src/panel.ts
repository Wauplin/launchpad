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
  <p class="x-muted">Your agents run in Launchpad on your computer. This panel lets them read, and when you allow it, use the tabs you choose.</p>
  <button class="x-btn x-pri" style="width:100%" id="local">Connect to this computer</button>
  <details id="other" style="width:100%"><summary class="x-btn x-link" style="list-style:none">Use Launchpad on another computer</summary>
    <form id="remote" style="display:grid;gap:8px;margin-top:8px;text-align:left"><label class="x-small x-faint">Launchpad address<input id="origin" class="x-field" style="margin-top:4px" type="url" required placeholder="https://your-machine.ts.net"></label><button class="x-btn" type="submit">Show pairing code</button></form>
  </details>
  <p id="lead" class="x-muted" hidden>Open Launchpad on a paired device → Settings → Devices, and approve this code:</p>
  <p id="code" class="x-code" hidden aria-live="polite"></p>
  <p id="status" class="x-small x-muted" role="status"></p>
  <p class="x-small x-faint">Nothing is shared until you send a message.</p>
</div></div>`;
const status = document.getElementById("status")!;
const local = document.getElementById("local") as HTMLButtonElement;
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
  await chrome.runtime.sendMessage({ type: "connect" });
  const app = await import("./app");
  app.mount();
}
local.onclick = () => {
  local.disabled = true;
  status.textContent = "Connecting…";
  void (async () => {
    const pair = await chrome.runtime.sendNativeMessage("dev.launchpad.browser", { type: "pair" });
    if (pair.error) throw new Error(pair.error);
    const link = new URL(pair.url);
    const origin = originUrl(link.origin);
    if (!["127.0.0.1", "localhost"].includes(link.hostname)) throw new Error("Native pairing must use this computer's loopback address");
    const code = new URLSearchParams(link.hash.slice(1)).get("code");
    if (!code) throw new Error("Launchpad returned an invalid pairing link");
    await json(origin, "/auth", { code, name: "Chrome on this computer" });
    await ready(origin);
  })().catch((e) => { status.textContent = `${String(e).replace(/^Error: /, "")}. Register the native host with extension/register-native-host.py, or use Launchpad on another computer to pair with a code.`; local.disabled = false; });
};
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
