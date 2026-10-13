// Launchpad service worker: installable app, offline shell and Web Push.
// - Hashed build assets (/assets/*) are cache-first: their names change with their content.
// - Navigations are network-first, falling back to the cached app shell when offline.
// - API, WebSocket and sign-in requests are never cached.
// Served with Cache-Control: no-cache, so the browser picks up new versions on its update checks.

const SHELL = "launchpad-shell-v1";
const ASSETS = "launchpad-assets-v1";
const MAX_ASSETS = 150;

self.addEventListener("install", (event) => {
  event.waitUntil((async () => {
    const cache = await caches.open(SHELL);
    // Only the signed-in app shell is worth keeping; never the sign-in page (no-store).
    const response = await fetch("/", { cache: "no-cache" }).catch(() => null);
    if (cacheableShell(response)) await cache.put("/", response);
    await self.skipWaiting();
  })());
});

self.addEventListener("activate", (event) => {
  event.waitUntil((async () => {
    for (const name of await caches.keys()) {
      if (name !== SHELL && name !== ASSETS) await caches.delete(name);
    }
    await self.clients.claim();
  })());
});

function cacheableShell(response) {
  return !!response && response.ok && response.type === "basic" && !(response.headers.get("Cache-Control") || "").includes("no-store")
    && (response.headers.get("Content-Type") || "").startsWith("text/html");
}

self.addEventListener("fetch", (event) => {
  const request = event.request;
  if (request.method !== "GET") return;
  const url = new URL(request.url);
  if (url.origin !== self.location.origin) return;
  const path = url.pathname;
  if (path.startsWith("/api/") || path === "/ws" || path === "/auth" || path === "/sw.js") return;
  if (request.mode === "navigate") {
    event.respondWith(navigate(request));
  } else if (path.startsWith("/assets/")) {
    event.respondWith(asset(request));
  }
});

async function navigate(request) {
  try {
    const response = await fetch(request);
    if (cacheableShell(response)) {
      const copy = response.clone();
      void caches.open(SHELL).then((cache) => cache.put("/", copy));
    }
    return response;
  } catch (error) {
    const shell = await caches.match("/", { cacheName: SHELL });
    if (shell) return shell;
    throw error;
  }
}

async function asset(request) {
  const cache = await caches.open(ASSETS);
  const hit = await cache.match(request);
  if (hit) return hit;
  const response = await fetch(request);
  if (response.ok && response.type === "basic") {
    await cache.put(request, response.clone());
    const keys = await cache.keys();
    for (const old of keys.slice(0, Math.max(0, keys.length - MAX_ASSETS))) await cache.delete(old);
  }
  return response;
}

self.addEventListener("push", (event) => {
  let data = {};
  try { data = event.data ? event.data.json() : {}; } catch { data = { title: "Launchpad", body: event.data ? event.data.text() : "" }; }
  event.waitUntil((async () => {
    if (typeof data.needs === "number" && "setAppBadge" in self.navigator) {
      try { await (data.needs > 0 ? self.navigator.setAppBadge(data.needs) : self.navigator.clearAppBadge()); } catch { /* Not supported here. */ }
    }
    await self.registration.showNotification(data.title || "Launchpad", {
      body: data.body || "",
      icon: "/icons/icon-192.png",
      badge: "/icons/badge-96.png",
      tag: data.tag || undefined,
      renotify: !!data.tag,
      data: { url: data.url || "/agents" },
    });
  })());
});

self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  const target = new URL(event.notification.data?.url || "/agents", self.location.origin);
  if (target.origin !== self.location.origin) return;
  const url = target.pathname + target.search + target.hash;
  event.waitUntil((async () => {
    const windows = await self.clients.matchAll({ type: "window", includeUncontrolled: true });
    const client = windows.find((c) => c.focused) || windows.find((c) => c.visibilityState === "visible") || windows[0];
    if (client) {
      // The page navigates with its router (web/src/lib/push.ts), keeping its state and socket.
      client.postMessage({ type: "launchpad-navigate", url });
      try { await client.focus(); } catch { /* Focus is best effort. */ }
      return;
    }
    await self.clients.openWindow(url);
  })());
});
