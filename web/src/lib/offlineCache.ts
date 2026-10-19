import { storageAllowed } from "./clientStorage.ts";
// Last known daemon data in IndexedDB, so a browser tab (a phone above all) draws at once from cache, with its age
// visible, then refreshes: the first WebSocket connection always resyncs every query. The desktop shell is local
// and does not use it. Only lists that the first screens need are kept.

import type { QueryClient } from "@tanstack/react-query";

const KEYS = ["status", "workspaces", "projects", "sessions", "predrafts", "suggestions", "inbox"];
const DB = "launchpad-cache";
const STORE = "queries";
type Snapshot = { at: number; entries: [string, unknown][] };

let restoredAt: number | null = null;
/** When the data shown was saved, while it comes from the cache (null once nothing restored). */
export const cachedAt = () => restoredAt;

function open(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB, 1);
    req.onupgradeneeded = () => req.result.createObjectStore(STORE);
    req.onsuccess = () => { req.result.onversionchange = () => req.result.close(); resolve(req.result); };
    req.onerror = () => reject(req.error);
  });
}

/** Puts the saved lists into the query cache. Never waits more than 300 ms. */
export async function restoreCache(qc: QueryClient): Promise<void> {
  try {
    const read = open().then((db) => new Promise<Snapshot | undefined>((resolve, reject) => {
      const req = db.transaction(STORE).objectStore(STORE).get("snapshot");
      req.onsuccess = () => { db.close(); resolve(req.result as Snapshot | undefined); };
      req.onerror = () => { db.close(); reject(req.error); };
    }));
    const snap = await Promise.race([read, new Promise<undefined>((r) => setTimeout(() => r(undefined), 300))]);
    if (!snap || !storageAllowed()) return;
    // Never replace data the daemon already sent.
    for (const [key, data] of snap.entries) if (!qc.getQueryState([key])?.dataUpdatedAt) qc.setQueryData([key], data, { updatedAt: snap.at });
    restoredAt = snap.at;
  } catch { /* No cache: load normally. */ }
}

/** Saves the lists a few seconds after they change. */
export function persistCache(qc: QueryClient) {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const save = () => {
    if (!storageAllowed()) return;
    const entries = KEYS.map((k) => [k, qc.getQueryData([k])] as [string, unknown]).filter(([, v]) => v !== undefined);
    void open().then((db) => { if (!storageAllowed()) { db.close(); return; } const tx = db.transaction(STORE, "readwrite"); tx.objectStore(STORE).put({ at: Date.now(), entries } satisfies Snapshot, "snapshot"); tx.oncomplete = () => db.close(); }).catch(() => {});
  };
  qc.getQueryCache().subscribe((event) => {
    if (event.type !== "updated" || !KEYS.includes(String(event.query.queryKey[0])) || event.query.queryKey.length !== 1) return;
    if (event.query.state.dataUpdatedAt > (restoredAt ?? 0)) restoredAt = null;
    clearTimeout(timer);
    timer = setTimeout(save, 3000);
  });
  window.addEventListener("pagehide", () => { clearTimeout(timer); save(); });
}
