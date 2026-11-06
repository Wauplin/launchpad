#!/usr/bin/env node
// Screenshots of the UI at several zoom levels and in the phone layout, with an overflow check.
//
// No dependencies: headless Chrome over the DevTools pipe. The pages come from this checkout's `web/dist` (rebuilt
// first when `web/src` is newer), served by a small in-process server that proxies the API to the daemon. So every
// worktree shoots its own UI, no server has to be started or stopped, and several runs can go at the same time.
//
//   node web/scripts/shots.mjs [options] <page>...          (or: cd web && pnpm shots …)
//
// A page is `/path` or `name=/path`. See `--help` and docs/development.md#screenshots.

import { spawn, spawnSync } from "node:child_process";
import { createServer, request as httpRequest } from "node:http";
import { connect } from "node:net";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { parseArgs } from "node:util";

const HELP = `Usage: node web/scripts/shots.mjs [options] <page>...

Pages: "/path" or "name=/path" (for example "/" "agents=/agents" "settings=/settings/workspace").
       Add steps to a page with --pages <file.json>: [{"name", "path", "click": ["css" or "text=Label"], "eval": "js", "wait": ms}].

Options:
  --views <list>     Views to shoot, comma-separated: zoom levels 100, 110, 120, 135, 150 (laptop) and "phone".
                     Default: 120. Example: --views 100,120,150,phone
  --demo             Demo mode (no daemon, no sign-in). --chapter <name> picks the tour chapter (default: explore).
  --daemon <url>     Daemon for real data (default http://127.0.0.1:$LAUNCHPAD_PORT or :7318). Never the production daemon.
  --token-file <f>   The daemon's token, used once to sign in (default ~/.local/share/launchpad-dev/token).
  --base <url>       Use a UI server that already runs (for example \`pnpm dev\` on :5173) instead of web/dist.
  --out <dir>        Output folder (default /tmp/lp-shots). Files are <out>/<prefix><view>-<name>.png.
  --prefix <text>    File name prefix (for example "wave3-").
  --wait <ms>        Extra wait after each page is idle (default 0; use ~1500 for syntax colors in Files).
  --timeout <ms>     Longest wait for a page to become idle (default 8000).
  --size <WxH>       Laptop viewport (default 1366x768). --phone-size <WxH> (default 390x844).
  --first-visit      Show page explanations and the welcome dialog, as on a first visit (hidden by default).
  --no-build         Do not rebuild web/dist, even when web/src is newer.
  --chrome <path>    Chrome binary (default: $CHROME, google-chrome, chromium, chromium-browser).
`;

const { values: opt, positionals } = parseArgs({
  allowPositionals: true,
  options: {
    views: { type: "string", default: "120" },
    demo: { type: "boolean", default: false },
    chapter: { type: "string", default: "" },
    daemon: { type: "string" },
    "token-file": { type: "string" },
    base: { type: "string" },
    out: { type: "string", default: "/tmp/lp-shots" },
    prefix: { type: "string", default: "" },
    wait: { type: "string", default: "0" },
    timeout: { type: "string", default: "8000" },
    size: { type: "string", default: "1366x768" },
    "phone-size": { type: "string", default: "390x844" },
    "first-visit": { type: "boolean", default: false },
    "no-build": { type: "boolean", default: false },
    pages: { type: "string" },
    chrome: { type: "string" },
    help: { type: "boolean", short: "h", default: false },
  },
});
if (opt.help) { process.stdout.write(HELP); process.exit(0); }

const WEB = path.resolve(import.meta.dirname, "..");
const DIST = path.join(WEB, "dist");
const ZOOMS = [100, 110, 120, 135, 150]; // web/src/lib/zoom.ts ZOOM_LEVELS: localStorage keeps the position.
const fail = (msg) => { console.error(`shots: ${msg}`); process.exit(2); };
const size = (s) => { const m = /^(\d+)x(\d+)$/.exec(s); if (!m) fail(`bad size ${s}`); return { width: +m[1], height: +m[2] }; };

// ---- Pages and views

const slug = (p) => p.replace(/^\/+/, "").replace(/[?#].*$/, "").replace(/[^a-zA-Z0-9]+/g, "-").replace(/-+$/, "") || "today";
const pages = positionals.map((arg) => {
  const eq = arg.indexOf("=");
  const [name, p] = eq > 0 && !arg.startsWith("/") ? [arg.slice(0, eq), arg.slice(eq + 1)] : [slug(arg), arg];
  if (!p.startsWith("/")) fail(`a page must start with "/": ${arg}`);
  return { name, path: p };
});
if (opt.pages) pages.push(...JSON.parse(fs.readFileSync(opt.pages, "utf8")).map((p) => ({ name: p.name ?? slug(p.path), ...p })));
if (!pages.length) fail(`no pages given\n\n${HELP}`);

const views = opt.views.split(",").map((v) => v.trim()).filter(Boolean).map((v) => {
  if (v === "phone") return { name: "phone", phone: true, ...size(opt["phone-size"]) };
  const zoom = ZOOMS.indexOf(Number(v.replace(/%$/, "")));
  if (zoom < 0) fail(`unknown view ${v} (use ${ZOOMS.join(", ")} or phone)`);
  return { name: `laptop${v.replace(/%$/, "")}`, zoom, ...size(opt.size) };
});

// ---- Sign-in (real data): one device per daemon, its cookie cached in ~/.cache/launchpad-shots

const daemon = new URL(opt.daemon ?? `http://127.0.0.1:${process.env.LAUNCHPAD_PORT || 7318}`);
let cookie = null;
if (!opt.demo) {
  if (daemon.port === "7317") fail("refusing the production daemon (port 7317): use the dev daemon, a scratch daemon or --demo");
  const alive = await fetch(new URL("/api/status", daemon)).then(() => true, () => false);
  if (!alive) fail(`no daemon answers on ${daemon.origin}. Start one (LAUNCHPAD_ENV=dev cargo run -p launchpad -- daemon) or use --demo`);
  cookie = await signIn();
}

async function signIn() {
  const cacheDir = path.join(os.homedir(), ".cache", "launchpad-shots");
  const cacheFile = path.join(cacheDir, `cookie-${daemon.hostname}-${daemon.port}.json`);
  const works = async (c) => c && (await fetch(new URL("/api/status", daemon), { headers: { cookie: `${c.name}=${c.value}` } })).ok;
  try { const c = JSON.parse(fs.readFileSync(cacheFile, "utf8")); if (await works(c)) return c; } catch { /* Sign in again. */ }
  const tokenFile = opt["token-file"] ?? path.join(os.homedir(), ".local/share/launchpad-dev/token");
  let token;
  try { token = fs.readFileSync(tokenFile, "utf8").trim(); } catch { fail(`cannot read the daemon token ${tokenFile} (use --token-file)`); }
  const pair = await fetch(new URL("/api/devices/pair", daemon), { method: "POST", headers: { authorization: `Bearer ${token}`, "x-launchpad-local-pair": "1" } });
  if (!pair.ok) fail(`pairing failed: ${pair.status} ${await pair.text()}`);
  const code = new URL((await pair.json()).url).hash.match(/code=([^&]+)/)?.[1];
  if (!code) fail("pairing returned no code");
  const res = await fetch(new URL("/auth", daemon), { method: "POST", headers: { "content-type": "application/json", origin: daemon.origin }, body: JSON.stringify({ code: decodeURIComponent(code), name: "Screenshots (headless)" }) });
  const set = res.headers.get("set-cookie")?.split(";")[0];
  if (!res.ok || !set) fail(`sign-in failed: ${res.status} ${await res.text()}`);
  const c = { name: set.slice(0, set.indexOf("=")), value: set.slice(set.indexOf("=") + 1) };
  fs.mkdirSync(cacheDir, { recursive: true, mode: 0o700 });
  fs.writeFileSync(cacheFile, JSON.stringify(c), { mode: 0o600 });
  console.log(`signed in to ${daemon.origin} as the device "Screenshots (headless)"`);
  return c;
}

// ---- UI server: this checkout's web/dist, the API proxied to the daemon

function newestMtime(p) {
  let st;
  try { st = fs.statSync(p); } catch { return 0; }
  if (!st.isDirectory()) return st.mtimeMs;
  return Math.max(0, ...fs.readdirSync(p).map((n) => newestMtime(path.join(p, n))));
}

function buildIfStale() {
  const sources = Math.max(...["src", "public", "index.html", "vite.config.ts", "package.json"].map((p) => newestMtime(path.join(WEB, p))));
  const built = newestMtime(path.join(DIST, "index.html"));
  if (built >= sources) return;
  if (opt["no-build"]) { console.warn(built ? "shots: web/dist is older than web/src (--no-build)" : "shots: web/dist is missing (--no-build)"); return; }
  // One build at a time per checkout: a second run waits for the first one's build.
  const lock = path.join(WEB, "node_modules", ".shots-build.lock");
  for (let i = 0; ; i++) {
    try { fs.mkdirSync(lock); break; } catch {
      if (i > 240) { fs.rmSync(lock, { recursive: true, force: true }); continue; }
      spawnSync("sleep", ["0.5"]);
    }
  }
  try {
    if (newestMtime(path.join(DIST, "index.html")) >= sources) return;
    console.log("shots: building web/dist (pnpm build)…");
    const r = spawnSync("pnpm", ["build"], { cwd: WEB, stdio: ["ignore", "ignore", "inherit"] });
    if (r.status !== 0) fail("pnpm build failed");
  } finally { fs.rmSync(lock, { recursive: true, force: true }); }
}

const TYPES = { ".html": "text/html; charset=utf-8", ".js": "text/javascript", ".mjs": "text/javascript", ".css": "text/css", ".json": "application/json", ".webmanifest": "application/manifest+json", ".svg": "image/svg+xml", ".png": "image/png", ".jpg": "image/jpeg", ".ico": "image/x-icon", ".woff2": "font/woff2", ".woff": "font/woff", ".wasm": "application/wasm", ".txt": "text/plain" };
const PROXIED = /^\/(api|auth|img|ws)(\/|$|\?)/;

function proxyHeaders(headers) {
  return { ...headers, host: daemon.host, ...(headers.origin ? { origin: daemon.origin } : {}) };
}

async function startServer() {
  if (!fs.existsSync(path.join(DIST, "index.html"))) fail("web/dist is missing: run (cd web && pnpm build)");
  const server = createServer((req, res) => {
    if (PROXIED.test(req.url) && opt.demo) {
      res.writeHead(404, { "content-type": "application/json" });
      res.end(JSON.stringify({ error: "demo mode: no daemon behind this server" }));
      return;
    }
    if (PROXIED.test(req.url)) {
      const up = httpRequest({ host: daemon.hostname, port: daemon.port, method: req.method, path: req.url, headers: proxyHeaders(req.headers) }, (r) => { res.writeHead(r.statusCode, r.headers); r.pipe(res); });
      up.on("error", () => { res.writeHead(502); res.end("daemon unreachable"); });
      req.pipe(up);
      return;
    }
    const rel = decodeURIComponent(new URL(req.url, "http://x").pathname);
    let file = path.join(DIST, path.normalize(rel));
    if (!file.startsWith(DIST) || !fs.existsSync(file) || fs.statSync(file).isDirectory()) file = path.join(DIST, "index.html");
    res.writeHead(200, { "content-type": TYPES[path.extname(file)] ?? "application/octet-stream", "cache-control": "no-store" });
    fs.createReadStream(file).pipe(res);
  });
  // WebSockets (the event bus, terminals): a raw byte pipe after the rewritten handshake.
  server.on("upgrade", (req, socket, head) => {
    if (opt.demo || !PROXIED.test(req.url)) return socket.destroy();
    const up = connect(Number(daemon.port), daemon.hostname, () => {
      const h = proxyHeaders(req.headers);
      up.write(`${req.method} ${req.url} HTTP/1.1\r\n${Object.entries(h).map(([k, v]) => `${k}: ${v}`).join("\r\n")}\r\n\r\n`);
      if (head?.length) up.write(head);
      up.pipe(socket); socket.pipe(up);
    });
    up.on("error", () => socket.destroy());
    socket.on("error", () => up.destroy());
  });
  await new Promise((ok) => server.listen(0, "127.0.0.1", ok));
  return server;
}

let server = null;
let base;
if (opt.base) base = new URL(opt.base);
else {
  buildIfStale();
  server = await startServer();
  base = new URL(`http://127.0.0.1:${server.address().port}`);
}

// ---- Chrome over the DevTools pipe (fd 3: commands in, fd 4: events out)

function findChrome() {
  for (const c of [opt.chrome, process.env.CHROME, "google-chrome", "google-chrome-stable", "chromium", "chromium-browser", "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"]) {
    if (!c) continue;
    if (c.includes("/")) { if (fs.existsSync(c)) return c; continue; }
    const r = spawnSync("which", [c], { encoding: "utf8" });
    if (r.status === 0) return r.stdout.trim();
  }
  fail("Chrome not found (use --chrome or $CHROME)");
}

const profile = fs.mkdtempSync(path.join(os.tmpdir(), "lp-shots-chrome-"));
const chrome = spawn(findChrome(), ["--headless=new", "--remote-debugging-pipe", `--user-data-dir=${profile}`, "--no-first-run", "--no-default-browser-check", "--disable-extensions", "--disable-gpu", "--mute-audio", "--disable-background-timer-throttling", "--disable-renderer-backgrounding", "about:blank"], { stdio: ["ignore", "ignore", "ignore", "pipe", "pipe"] });
const toChrome = chrome.stdio[3];
const fromChrome = chrome.stdio[4];
let nextId = 1;
const waiting = new Map();
const listeners = new Set();
let buffer = "";
fromChrome.on("data", (chunk) => {
  buffer += chunk.toString("utf8");
  let end;
  while ((end = buffer.indexOf("\0")) >= 0) {
    const msg = JSON.parse(buffer.slice(0, end));
    buffer = buffer.slice(end + 1);
    if (msg.id && waiting.has(msg.id)) {
      const { ok, ko, method } = waiting.get(msg.id);
      waiting.delete(msg.id);
      if (msg.error) ko(new Error(`${method}: ${msg.error.message}`)); else ok(msg.result);
    } else for (const l of listeners) l(msg);
  }
});
const send = (method, params = {}, sessionId) => new Promise((ok, ko) => {
  const id = nextId++;
  waiting.set(id, { ok, ko, method });
  toChrome.write(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }) + "\0");
});

let cleaned = false;
function cleanup() {
  if (cleaned) return;
  cleaned = true;
  try { chrome.kill("SIGKILL"); } catch { /* Gone. */ }
  server?.close();
  fs.rmSync(profile, { recursive: true, force: true });
}
process.on("exit", cleanup);
for (const s of ["SIGINT", "SIGTERM"]) process.on(s, () => { cleanup(); process.exit(130); });

// ---- One view: its own browser context (own storage), pages one after the other with in-app navigation

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// Runs in the page: problems a reader would notice (horizontal scroll, bars whose items leave the screen or overlap).
const OVERFLOW = `(() => {
  const W = innerWidth, out = [];
  if (document.documentElement.scrollWidth > W + 1) out.push("page scrolls sideways (" + document.documentElement.scrollWidth + " > " + W + ")");
  for (const bar of document.querySelectorAll("header, footer, nav")) {
    const kids = [...bar.children].filter((k) => k.getBoundingClientRect().width > 0);
    for (const k of kids) { const r = k.getBoundingClientRect(); if (r.right > W + 1) out.push(bar.tagName.toLowerCase() + " item leaves the screen: " + (k.className.toString().slice(0, 50) || k.tagName)); }
    if (getComputedStyle(bar).flexDirection === "column") continue;
    for (let i = 1; i < kids.length; i++) { const a = kids[i - 1].getBoundingClientRect(), b = kids[i].getBoundingClientRect(); if (Math.abs(a.top - b.top) < 4 && a.right > b.left + 2) out.push(bar.tagName.toLowerCase() + " items overlap"); }
  }
  return [...new Set(out)];
})()`;

async function shootView(view) {
  const { browserContextId } = await send("Target.createBrowserContext", { disposeOnDetach: true });
  const { targetId } = await send("Target.createTarget", { url: "about:blank", browserContextId });
  const { sessionId } = await send("Target.attachToTarget", { targetId, flatten: true });
  const s = (method, params) => send(method, params, sessionId);

  // Requests in flight (long-lived sockets and event streams excluded) and errors, for this page only.
  const inflight = new Set();
  let lastActivity = Date.now();
  let errors = [];
  const onEvent = (msg) => {
    if (msg.sessionId !== sessionId) return;
    const p = msg.params;
    switch (msg.method) {
      case "Network.requestWillBeSent": if (p.type !== "EventSource" && p.type !== "WebSocket") { inflight.add(p.requestId); lastActivity = Date.now(); } break;
      case "Network.loadingFinished": case "Network.loadingFailed": inflight.delete(p.requestId); lastActivity = Date.now(); break;
      // In demo mode only the onboarding record reaches the server; the stub answers 404 (expected).
      case "Network.responseReceived": if (p.response.status >= 400 && !/\/favicon/.test(p.response.url) && !(opt.demo && PROXIED.test(new URL(p.response.url).pathname))) errors.push(`HTTP ${p.response.status} ${p.response.url.replace(base.origin, "")}`); break;
      case "Runtime.exceptionThrown": errors.push(`exception: ${p.exceptionDetails.exception?.description?.split("\n")[0] ?? p.exceptionDetails.text}`); break;
      case "Runtime.consoleAPICalled": if (p.type === "error") errors.push(`console.error: ${p.args.map((a) => a.value ?? a.description ?? "").join(" ").slice(0, 200)}`); break;
    }
  };
  listeners.add(onEvent);
  await Promise.all([s("Page.enable"), s("Runtime.enable"), s("Network.enable")]);
  await s("Emulation.setDeviceMetricsOverride", { width: view.width, height: view.height, deviceScaleFactor: view.phone ? 2 : 1, mobile: !!view.phone });
  if (view.phone) await s("Emulation.setTouchEmulationEnabled", { enabled: true, maxTouchPoints: 5 });
  if (cookie) await s("Network.setCookie", { ...cookie, url: base.origin, httpOnly: true });
  const storage = { "launchpad-zoom": String(view.zoom ?? 2), "launchpad-layout": view.phone ? "phone" : "desktop" };
  await s("Page.addScriptToEvaluateOnNewDocument", { source: `try { for (const [k, v] of Object.entries(${JSON.stringify(storage)})) localStorage.setItem(k, v); } catch {} ${opt["first-visit"] ? "" : "window.__lpScreenshots = true;"}` });

  const timeout = Number(opt.timeout);
  const settle = async () => {
    const start = Date.now();
    while (Date.now() - start < timeout && (inflight.size > 0 || Date.now() - lastActivity < 400)) await sleep(50);
    await s("Runtime.evaluate", { expression: "document.fonts.ready.then(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))))", awaitPromise: true });
    if (Number(opt.wait) > 0) await sleep(Number(opt.wait));
  };
  const evaluate = async (expression) => {
    const r = await s("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
    if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description?.split("\n")[0] ?? r.exceptionDetails.text);
    return r.result.value;
  };

  const results = [];
  let loaded = false;
  for (const page of pages) {
    errors = [];
    const line = { file: path.join(opt.out, `${opt.prefix}${view.name}-${page.name}.png`) };
    try {
      if (!loaded) {
        // The first page loads the app; demo mode starts from its query parameter, then keeps its flag in window.name.
        const url = new URL(page.path, base);
        // The phone shell has its own demo entry (`?demo=phone`); the tour chapters are desktop only.
        if (opt.demo) url.searchParams.set("demo", opt.chapter || (view.phone ? "phone" : "1"));
        await s("Page.navigate", { url: url.href });
        loaded = true;
      } else {
        await evaluate(`history.pushState({}, "", ${JSON.stringify(page.path)}); dispatchEvent(new PopStateEvent("popstate"))`);
      }
      await settle();
      for (const step of [].concat(page.click ?? [])) {
        const ok = await evaluate(step.startsWith("text=")
          ? `(() => { const t = ${JSON.stringify(step.slice(5))}; const el = [...document.querySelectorAll("button, a, [role=button], [role=tab], summary")].find((e) => e.offsetParent && e.innerText.trim() === t); el?.click(); return !!el; })()`
          : `(() => { const el = document.querySelector(${JSON.stringify(step)}); el?.click(); return !!el; })()`);
        if (!ok) errors.push(`click: nothing matches ${step}`);
        await settle();
      }
      if (page.eval) { await evaluate(page.eval); await settle(); }
      if (page.wait) await sleep(page.wait);
      line.problems = await evaluate(OVERFLOW);
      const { data } = await s("Page.captureScreenshot", { format: "png" });
      fs.writeFileSync(line.file, Buffer.from(data, "base64"));
      line.url = (await evaluate("location.pathname + location.search")) ?? "";
    } catch (e) {
      errors.push(`failed: ${e.message}`);
    }
    line.errors = [...new Set(errors)];
    results.push(line);
  }
  listeners.delete(onEvent);
  await send("Target.disposeBrowserContext", { browserContextId }).catch(() => {});
  return results;
}

// ---- Run: views in parallel, one line per screenshot

fs.mkdirSync(opt.out, { recursive: true });
await send("Browser.getVersion").catch(() => fail("Chrome did not start"));
console.log(`shots: ${opt.demo ? "demo mode" : `daemon ${daemon.origin}`}, UI from ${opt.base ? base.origin : "web/dist"}, ${views.length} view(s) × ${pages.length} page(s) → ${opt.out}`);
const all = (await Promise.all(views.map(shootView))).flat();
let issues = 0;
for (const r of all) {
  const notes = [...(r.problems ?? []), ...r.errors];
  issues += notes.length ? 1 : 0;
  console.log(`${notes.length ? "!!" : "ok"} ${r.file}${r.url ? `  (${r.url})` : ""}${notes.length ? `\n     ${notes.join("\n     ")}` : ""}`);
}
console.log(`shots: ${all.length} screenshot(s), ${issues} with problems`);
// Close Chrome first, so it does not write into its profile while the profile is removed.
const exited = new Promise((r) => chrome.once("exit", r));
await send("Browser.close").catch(() => {});
await Promise.race([exited, sleep(3000)]);
cleanup();
process.exit(issues ? 1 : 0);
