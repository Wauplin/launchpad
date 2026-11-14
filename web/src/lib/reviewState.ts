// Review progress kept by the daemon (`/api/review-state`), so every browser and device sees the same
// viewed markers and unsent drafts. Keys are namespaced: `viewed:`, `predraft:`, `suggestion:`.
// Writes are debounced; other windows refresh on the `review-state` bus event. Pure UI state (layout,
// collapsed groups, selection, diff view mode) stays in local storage.

import { useQuery, useQueryClient, type QueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useRef, useState } from "react";
import { ApiError, api } from "./api";
import { storageAllowed } from "./clientStorage";
import { useSaveStatus } from "./saveStatus";
import { onEvent } from "./ws";

type Stored = { key: string; value: unknown; updated_at: number | null };

/** Identifies this window, so it ignores the echo of its own writes. */
const ORIGIN = crypto.randomUUID?.() ?? `${Date.now()}-${Math.random()}`;
const DEBOUNCE_MS = 400;

type Pending = { value: unknown; base: number; conflict?: boolean };
const backupKey = `launchpad-review-pending:${sessionStorage.getItem("launchpad-review-client") ?? ORIGIN}`;
sessionStorage.setItem("launchpad-review-client", backupKey.split(":").at(-1)!);
const pending = new Map<string, Pending>();
try { for (const [key, entry] of Object.entries(JSON.parse(localStorage.getItem(backupKey) ?? "{}"))) pending.set(key, entry as Pending); } catch { /* Unreadable backup. */ }
const localListeners = new Map<string, Set<() => void>>();
const running = new Set<string>();
const timers = new Map<string, ReturnType<typeof setTimeout>>();
let setupDone = false;

const url = (key: string) => `/api/review-state?key=${encodeURIComponent(key)}`;
const queryKey = (key: string) => ["review-state", key];
function backup() {
  if (!storageAllowed()) return;
  try {
    if (pending.size) localStorage.setItem(backupKey, JSON.stringify(Object.fromEntries(pending)));
    else localStorage.removeItem(backupKey);
    useSaveStatus.getState().set("review-storage");
  } catch { useSaveStatus.getState().set("review-storage", "Local draft backup unavailable. Keep this window open until saved."); }
}

export async function flushReviewState(qc: QueryClient, key: string) {
  if (running.has(key) || !storageAllowed()) return;
  clearTimeout(timers.get(key));
  timers.delete(key);
  running.add(key);
  try {
    while (pending.has(key)) {
      const entry = pending.get(key)!;
      if (entry.conflict) break;
      try {
        const res = await api.put<{ updated_at: number }>(url(key), { value: entry.value, origin: ORIGIN, base_revision: entry.base });
        const latest = pending.get(key);
        if (latest === entry) {
          // Install only the newest acknowledged value, never an older in-flight edit.
          qc.setQueryData<Stored>(queryKey(key), { key, value: entry.value, updated_at: res.updated_at });
          pending.delete(key);
          useSaveStatus.getState().set(key);
        } else if (latest) latest.base = res.updated_at;
        backup();
      } catch (e) {
        if (!storageAllowed()) break;
        const latest = pending.get(key)!;
        if (e instanceof ApiError && e.status === 409) {
          latest.conflict = true;
          useSaveStatus.getState().set(key, "Review draft changed elsewhere. Choose which version to keep.", [
            { label: "Load newer", run: async () => {
              const saved = await api.get<Stored>(url(key));
              pending.delete(key); backup(); useSaveStatus.getState().set(key);
              qc.setQueryData(queryKey(key), saved);
            } },
            { label: "Keep mine", run: async () => {
              const saved = await api.get<Stored>(url(key));
              const current = pending.get(key);
              if (current) { current.base = saved.updated_at ?? 0; current.conflict = false; backup(); await flushReviewState(qc, key); }
            } },
          ]);
        } else useSaveStatus.getState().set(key, "Review draft not saved to server. Kept on this device; retrying.");
        backup();
        break;
      }
    }
  } finally { running.delete(key); }
}
export function scheduleReviewState(qc: QueryClient, key: string, value: unknown, isEmpty: boolean) {
  const previous = pending.get(key);
  pending.set(key, { value: isEmpty ? null : value, base: previous?.base ?? qc.getQueryData<Stored>(queryKey(key))?.updated_at ?? 0, conflict: previous?.conflict });
  backup();
  for (const notify of localListeners.get(key) ?? []) notify();
  clearTimeout(timers.get(key));
  timers.set(key, setTimeout(() => void flushReviewState(qc, key), DEBOUNCE_MS));
}
function setup(qc: QueryClient) {
  if (setupDone) return;
  setupDone = true;
  const flushAll = () => { for (const key of pending.keys()) void flushReviewState(qc, key); };
  onEvent((ev) => {
    if (ev.topic !== "review-state") return;
    const p = ev.payload as { key: string | null; origin: string | null };
    if (p.origin === ORIGIN) return;
    void qc.invalidateQueries({ queryKey: p.key ? queryKey(p.key) : ["review-state"] });
  });
  window.addEventListener("pagehide", flushAll);
  window.addEventListener("online", flushAll);
  window.addEventListener("launchpad-reconnected", flushAll);
  setInterval(flushAll, 10_000);
  // Replay recovered conflicts as conditional saves so their resolution controls reappear.
  for (const entry of pending.values()) entry.conflict = false;
  flushAll();
}

export const viewedKey = (id: string, number: number, head: string) => `viewed:${id}#${number}@${head}`;

/**
 * One review state value shared through the daemon. `empty` is the value when nothing is stored;
 * setting a value equal to it deletes the entry. `remote` increases when another window or device
 * replaced the value after the first load.
 */
export function useReviewState<T>(key: string, empty: T) {
  const qc = useQueryClient();
  setup(qc);
  const q = useQuery({ queryKey: queryKey(key), queryFn: ({ signal }) => api.get<Stored>(url(key), signal), staleTime: Infinity });
  const emptyJson = JSON.stringify(empty);
  const [value, setLocal] = useState<T>(() => (pending.get(key)?.value as T | undefined) ?? (q.data?.value as T | null | undefined) ?? empty);
  const [remote, setRemote] = useState(0);
  const loadedAt = useRef<number | null | undefined>(q.data ? q.data.updated_at : undefined);
  const current = useRef(value);
  current.current = value;
  useEffect(() => {
    if (!q.data || pending.has(key)) return;
    const first = loadedAt.current === undefined;
    if (!first && loadedAt.current === q.data.updated_at) return;
    loadedAt.current = q.data.updated_at;
    const next = (q.data.value as T | null) ?? empty;
    if (JSON.stringify(next) === JSON.stringify(current.current)) return;
    setLocal(next);
    if (!first) setRemote((n) => n + 1);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [q.data, key]);
  // Send an unsent edit right away when the view closes.
  useEffect(() => () => { void flushReviewState(qc, key); }, [key]);
  const set = useCallback((next: T | ((old: T) => T)) => {
    const v = typeof next === "function" ? (next as (old: T) => T)((pending.get(key)?.value as T | undefined) ?? current.current) : next;
    if (JSON.stringify(v) === JSON.stringify(current.current)) return;
    current.current = v;
    setLocal(v);
    scheduleReviewState(qc, key, v, JSON.stringify(v) === emptyJson);
  }, [qc, key, emptyJson]);
  useEffect(() => {
    const listeners = localListeners.get(key) ?? new Set<() => void>();
    localListeners.set(key, listeners);
    const sync = () => { const entry = pending.get(key); if (entry) { current.current = (entry.value as T | null) ?? JSON.parse(emptyJson) as T; setLocal(current.current); } };
    listeners.add(sync);
    sync();
    return () => { listeners.delete(sync); if (!listeners.size) localListeners.delete(key); };
  }, [key, emptyJson]);
  /** Deletes the stored value now, for example after the draft was posted. */
  const clear = useCallback(() => {
    current.current = JSON.parse(emptyJson) as T;
    setLocal(current.current);
    scheduleReviewState(qc, key, null, true);
    return flushReviewState(qc, key);
  }, [qc, key, emptyJson]);
  return { value, set, clear, loaded: q.isSuccess || q.isError, remote };
}
