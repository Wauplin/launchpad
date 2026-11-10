// Launchpad's service worker: an offline app shell only (registered in src/main.tsx). Page loads go to the network
// first and fall back to the last app page (index.html) when offline; the hashed `/assets/*` files the page loads
// are kept so that page can start. Nothing else is cached: no API answers, hooks, sockets or non-GET requests.
// No Web Push and no notifications.

const CACHE = "launchpad-shell-1";
const SHELL = "/index.html";
const SKIP = ["/api/", "/hooks/", "/auth", "/git/", "/ws"];

self.addEventListener("install", () => self.skipWaiting());

self.addEventListener("activate", (event) => {
  event.waitUntil((async () => {
    const names = await caches.keys();
    await Promise.all(names.filter((n) => n.startsWith("launchpad-") && n !== CACHE).map((n) => caches.delete(n)));
    await self.clients.claim();
  })());
});

self.addEventListener("fetch", (event) => {
  const req = event.request;
  if (req.method !== "GET") return;
  const url = new URL(req.url);
  if (url.origin !== self.location.origin || SKIP.some((p) => url.pathname === p || url.pathname.startsWith(p))) return;
  if (req.mode === "navigate") event.respondWith(page(req));
  else if (url.pathname.startsWith("/assets/")) event.respondWith(asset(req));
});

async function page(req) {
  try {
    const res = await fetch(req);
    await keepShell(res.clone());
    return res;
  } catch (err) {
    const cached = await (await caches.open(CACHE)).match(SHELL);
    if (cached) return cached;
    throw err;
  }
}

// Only the app page itself: the sign-in page (no-store) and other answers stay out of the cache.
async function keepShell(res) {
  if (!res.ok || res.type !== "basic" || res.redirected) return;
  if (!(res.headers.get("Content-Type") ?? "").startsWith("text/html") || /no-store/.test(res.headers.get("Cache-Control") ?? "")) return;
  const html = await res.text();
  if (!html.includes('id="root"')) return;
  const cache = await caches.open(CACHE);
  const previous = await cache.match(SHELL);
  if (previous && (await previous.text()) === html) return;
  // A new build: drop the assets the new page does not name (lazy chunks come back as the page loads them).
  const named = new Set([...html.matchAll(/["'](\/assets\/[^"']+)["']/g)].map((m) => m[1]));
  for (const key of await cache.keys()) {
    const path = new URL(key.url).pathname;
    if (path.startsWith("/assets/") && !named.has(path)) await cache.delete(key);
  }
  await cache.put(SHELL, new Response(html, { headers: { "Content-Type": "text/html; charset=utf-8" } }));
}

// Hashed names never change: the cache answers first, else the network (and the answer is kept).
async function asset(req) {
  const cache = await caches.open(CACHE);
  const cached = await cache.match(req);
  if (cached) return cached;
  const res = await fetch(req);
  if (res.ok && res.type === "basic") await cache.put(req, res.clone());
  return res;
}
