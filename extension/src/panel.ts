// Pair before loading shared stores and their background writers.
import "./styles.css";

const root = document.getElementById("root")!;
root.innerHTML = `<main class="m-4 grid gap-4"><h1 class="text-xl font-bold">Launchpad</h1><p>Ask your agents about a page you choose. Read-only access, using your Launchpad profiles.</p><button class="btn btn-primary" id="local">Connect to Launchpad on this computer</button><details><summary>Connect to another computer</summary><form id="remote" class="mt-3 grid gap-3"><label>Launchpad URL<input id="origin" class="field-input mt-1 w-full" type="url" required placeholder="https://your-machine.ts.net"></label><button class="btn" type="submit">Show pairing code</button></form></details><p id="code" class="text-2xl font-bold tracking-widest" aria-live="polite"></p><p id="status" role="status"></p></main>`;
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
  })().catch((e) => { status.textContent = `${String(e)}. Install the native host using extension/register-native-host.py, or use a pairing code.`; local.disabled = false; });
};
document.getElementById("remote")!.onsubmit = (event) => {
  event.preventDefault();
  const attempt = ++polling;
  void (async () => {
    const origin = originUrl((document.getElementById("origin") as HTMLInputElement).value.trim());
    if (!await chrome.permissions.request({ origins: [`${origin}/*`] })) throw new Error("Launchpad connection permission was declined");
    const pair = await json(origin, "/auth/reverse/start", { name: "Chrome extension" });
    document.getElementById("code")!.textContent = `${pair.code.slice(0, 4)} ${pair.code.slice(4)}`;
    status.textContent = "Approve this code in Launchpad → Settings → Devices on a paired device. Keep this panel open.";
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
  if (daemon) { try { await ready(originUrl(String(daemon))); } catch { status.textContent = "Launchpad is offline or this device was revoked. Reconnect when it is available."; } }
});
