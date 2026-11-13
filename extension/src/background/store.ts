// What the worker keeps. Credentials live only here: the refresh token and the device secret in chrome.storage.local,
// the access token in chrome.storage.session; both areas are closed to content scripts (`lockStorage`).

import type { HomeInfo } from "../shared/messages";

export type Local = {
  /** Hugging Face refresh token. */
  "hf.refresh"?: string;
  /** The Hugging Face account of the sign-in. */
  "hf.user"?: { name: string; avatar?: string };
  /** The owner's OAuth client ID, set in the panel's Settings. */
  "hf.client_id"?: string;
  /** Where the extension is connected, and its device secret (`X-Launchpad-Device`). */
  "home"?: HomeInfo;
  "device"?: string;
  /** The extension's current workspace. */
  "workspace"?: string;
  /** The event stream's position, for `?boot=&since=` after the worker restarted. */
  "events.position"?: { boot: string; bus: number };
};
export type Session = {
  "hf.access"?: { token: string; expires_at: number };
  /** A manual pairing in progress. */
  "pairing"?: { origin: string; code: string; secret: string; expires_at: number; device_name: string };
};

export async function local<K extends keyof Local>(key: K): Promise<Local[K]> {
  return (await chrome.storage.local.get(key))[key] as Local[K];
}
export async function setLocal(values: Partial<Local>) {
  await chrome.storage.local.set(values);
}
export async function removeLocal(...keys: (keyof Local)[]) {
  await chrome.storage.local.remove(keys);
}
export async function session<K extends keyof Session>(key: K): Promise<Session[K]> {
  return (await chrome.storage.session.get(key))[key] as Session[K];
}
export async function setSession(values: Partial<Session>) {
  await chrome.storage.session.set(values);
}
export async function removeSession(...keys: (keyof Session)[]) {
  await chrome.storage.session.remove(keys);
}

/** Content scripts read neither area (session is closed by default; local is not). */
export async function lockStorage() {
  for (const area of [chrome.storage.local, chrome.storage.session]) {
    try { await area.setAccessLevel({ accessLevel: "TRUSTED_CONTEXTS" }); } catch (e) { console.warn("storage access level", e); }
  }
}
