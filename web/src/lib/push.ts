// Installable app and Web Push on this browser. The service worker is `web/public/sw.js`; the daemon
// keeps one subscription per endpoint, owned by this paired browser (`/api/push`). Revoking the
// browser in Settings → Devices deletes its subscriptions.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "./api";

declare global {
  interface Window { __launchpadDesktop?: boolean }
}

/** The Tauri desktop shell gets host notifications from the daemon; it never registers a worker. */
export function isDesktopShell(): boolean {
  return (typeof window.__launchpadDesktop !== "undefined" && !!window.__launchpadDesktop) || "__TAURI_INTERNALS__" in window;
}

function isIos(): boolean {
  return /iPad|iPhone|iPod/.test(navigator.userAgent) || (navigator.platform === "MacIntel" && navigator.maxTouchPoints > 1);
}

export type PushSupport =
  | "supported"
  /** iOS and iPadOS only offer Web Push to web apps added to the Home Screen. */
  | "ios-home-screen"
  | "desktop"
  /** Service workers need HTTPS (or http://127.0.0.1). */
  | "insecure"
  | "unsupported";

export function pushSupport(): PushSupport {
  if (isDesktopShell()) return "desktop";
  if (!window.isSecureContext) return "insecure";
  if (!("serviceWorker" in navigator) || !("PushManager" in window) || !("Notification" in window)) return isIos() ? "ios-home-screen" : "unsupported";
  return "supported";
}

/** Registers the service worker (installable app, offline shell, push) and follows notification clicks. */
export function registerServiceWorker(navigate: (url: string) => void) {
  if (isDesktopShell() || !window.isSecureContext || !("serviceWorker" in navigator)) return;
  navigator.serviceWorker.addEventListener("message", (event: MessageEvent) => {
    const data = event.data as { type?: string; url?: string } | null;
    if (data?.type === "launchpad-navigate" && typeof data.url === "string" && data.url.startsWith("/")) navigate(data.url);
  });
  window.addEventListener("load", () => {
    void navigator.serviceWorker.register("/sw.js", { scope: "/" }).catch((error) => console.warn("service worker:", error));
  });
}

/** The installed app's icon badge follows the Needs you count where the platform supports it. */
export function setAppBadge(count: number) {
  const nav = navigator as Navigator & { setAppBadge?: (n?: number) => Promise<void>; clearAppBadge?: () => Promise<void> };
  if (isDesktopShell() || !nav.setAppBadge) return;
  void (count > 0 ? nav.setAppBadge(count) : nav.clearAppBadge?.())?.catch(() => {});
}

type ServerState = { public_key: string; subscribed: boolean; titles: boolean };
export type PushState = {
  support: PushSupport;
  permission: NotificationPermission | "unsupported";
  /** This browser's subscription is registered with the daemon. */
  subscribed: boolean;
  titles: boolean;
};

const b64url = (buffer: ArrayBuffer | null) => {
  if (!buffer) return "";
  let s = "";
  for (const byte of new Uint8Array(buffer)) s += String.fromCharCode(byte);
  return btoa(s).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
};
const fromB64url = (s: string) => Uint8Array.from(atob(s.replaceAll("-", "+").replaceAll("_", "/")), (c) => c.charCodeAt(0));

async function registration(): Promise<ServiceWorkerRegistration> {
  const existing = await navigator.serviceWorker.getRegistration("/");
  if (existing) return existing;
  await navigator.serviceWorker.register("/sw.js", { scope: "/" });
  return navigator.serviceWorker.ready;
}

async function currentSubscription(): Promise<PushSubscription | null> {
  const reg = await navigator.serviceWorker.getRegistration("/");
  return reg ? reg.pushManager.getSubscription() : null;
}

export async function readPushState(): Promise<PushState> {
  const support = pushSupport();
  if (support !== "supported") return { support, permission: "unsupported", subscribed: false, titles: false };
  const sub = await currentSubscription();
  const server = await api.get<ServerState>(`/api/push${sub ? `?endpoint=${encodeURIComponent(sub.endpoint)}` : ""}`);
  return { support, permission: Notification.permission, subscribed: !!sub && server.subscribed, titles: server.titles };
}

async function register(sub: PushSubscription, titles: boolean) {
  await api.post("/api/push/subscribe", { endpoint: sub.endpoint, keys: { p256dh: b64url(sub.getKey("p256dh")), auth: b64url(sub.getKey("auth")) }, titles });
}

/** Asks for permission (after a click), subscribes with the daemon's VAPID key and registers it. */
export async function enablePush(titles: boolean) {
  const permission = await Notification.requestPermission();
  if (permission !== "granted") throw new Error(permission === "denied" ? "Notifications are blocked for this site. Allow them in the browser's site settings." : "Notification permission was not granted.");
  const { public_key } = await api.get<ServerState>("/api/push");
  const reg = await registration();
  let sub = await reg.pushManager.getSubscription();
  // A subscription made for another key (for example an older data folder) cannot be used.
  if (sub && b64url(sub.options.applicationServerKey) !== public_key) { await sub.unsubscribe(); sub = null; }
  sub ??= await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: fromB64url(public_key) });
  await register(sub, titles);
}

export async function disablePush() {
  const sub = await currentSubscription();
  if (!sub) return;
  await api.post("/api/push/unsubscribe", { endpoint: sub.endpoint });
  await sub.unsubscribe();
}

export async function setPushTitles(titles: boolean) {
  const sub = await currentSubscription();
  if (sub) await register(sub, titles);
}

export async function sendTestPush() {
  return api.post<{ sent: number }>("/api/push/test");
}

export function usePushState() {
  return useQuery({ queryKey: ["push"], queryFn: readPushState, staleTime: 60_000, enabled: !isDesktopShell() });
}

export function useRefreshPush() {
  const qc = useQueryClient();
  return () => Promise.all([qc.invalidateQueries({ queryKey: ["push"] }), qc.invalidateQueries({ queryKey: ["devices"] })]);
}
