// Review progress kept by the daemon (`/api/review-state`), so every browser and device sees the same
// viewed markers and unsent drafts. Keys are namespaced: `viewed:`, `reply:`, `predraft:`, `suggestion:`.
// Writes are debounced; other windows refresh on the `review-state` bus event. Pure UI state (layout,
// collapsed groups, selection, diff view mode) stays in local storage.

import { useQuery, useQueryClient, type QueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "./api";
import { onEvent } from "./ws";

type Stored = { key: string; value: unknown; updated_at: number | null };

/** Identifies this window, so it ignores the echo of its own writes. */
const ORIGIN = crypto.randomUUID?.() ?? `${Date.now()}-${Math.random()}`;
const DEBOUNCE_MS = 400;

/** Unsent writes per key; a newer edit replaces the older one. */
type Pending = { timer: ReturnType<typeof setTimeout> | null; value: unknown; version: number; flush: () => void };
const pending = new Map<string, Pending>();
let version = 0;
let setupDone = false;

const url = (key: string) => `/api/review-state?key=${encodeURIComponent(key)}`;
const queryKey = (key: string) => ["review-state", key];

function send(qc: QueryClient, key: string, value: unknown, isEmpty: boolean) {
  const v = ++version;
  const entry = pending.get(key);
  if (entry?.timer) clearTimeout(entry.timer);
  pending.set(key, { timer: null, value, version: v, flush: () => {} });
  return api.put(url(key), { value: isEmpty ? null : value, origin: ORIGIN })
    .then(() => qc.setQueryData<Stored>(queryKey(key), { key, value: isEmpty ? null : value, updated_at: Date.now() }))
    .catch((e) => console.warn(`review state ${key}: ${e}`))
    .finally(() => { if (pending.get(key)?.version === v) pending.delete(key); });
}

function schedule(qc: QueryClient, key: string, value: unknown, isEmpty: boolean) {
  const entry = pending.get(key);
  if (entry?.timer) clearTimeout(entry.timer);
  const v = ++version;
  const timer = setTimeout(() => { if (pending.get(key)?.version === v) void send(qc, key, value, isEmpty); }, DEBOUNCE_MS);
  pending.set(key, { timer, value, version: v, flush: () => void send(qc, key, value, isEmpty) });
}

function flush(key: string) {
  const e = pending.get(key);
  if (e?.timer) { clearTimeout(e.timer); e.timer = null; e.flush(); }
}

function flushAll() {
  [...pending.keys()].forEach(flush);
}

function setup(qc: QueryClient) {
  if (setupDone) return;
  setupDone = true;
  onEvent((ev) => {
    if (ev.topic !== "review-state") return;
    const p = ev.payload as { key: string | null; origin: string | null };
    if (p.origin === ORIGIN) return;
    void qc.invalidateQueries({ queryKey: p.key ? queryKey(p.key) : ["review-state"] });
  });
  window.addEventListener("pagehide", flushAll);
  void importLegacy();
}

/** Moves review state that older builds kept in this browser's local storage to the daemon, once. */
async function importLegacy() {
  const entries: Record<string, unknown> = {};
  const remove: string[] = [];
  try {
    const viewed = localStorage.getItem("diff-viewed");
    if (viewed) {
      remove.push("diff-viewed");
      for (const [k, on] of Object.entries(JSON.parse(viewed) as Record<string, boolean>)) {
        const [id, number, head, ...rest] = k.split(":");
        if (!on || !rest.length) continue;
        const key = viewedKey(id, Number(number), head);
        entries[key] = [...((entries[key] as string[] | undefined) ?? []), rest.join(":")];
      }
    }
    for (let i = 0; i < localStorage.length; i++) {
      const k = localStorage.key(i)!;
      const value = localStorage.getItem(k) ?? "";
      if (k.startsWith("github-reply:")) {
        const [, repo, number] = k.split(":");
        if (value.trim()) entries[replyKey(repo, Number(number))] = value;
        remove.push(k);
      } else if (k.startsWith("predraft-draft:")) {
        try { entries[`predraft:${k.slice("predraft-draft:".length)}`] = JSON.parse(value); } catch { /* Drop unreadable drafts. */ }
        remove.push(k);
      } else if (k.startsWith("suggestion-edit:")) {
        const rest = k.slice("suggestion-edit:".length);
        const [id, field] = rest.endsWith(":event") ? [rest.slice(0, -":event".length), "event"] : [rest, "body"];
        const key = `suggestion:${id}`;
        entries[key] = { ...((entries[key] as object | undefined) ?? {}), [field]: value };
        remove.push(k);
      }
    }
  } catch (e) {
    console.warn(`review state import: ${e}`);
    return;
  }
  if (!remove.length) return;
  try {
    if (Object.keys(entries).length) await api.post("/api/review-state/import", entries);
    remove.forEach((k) => localStorage.removeItem(k));
  } catch (e) {
    console.warn(`review state import: ${e}`);
  }
}

export const viewedKey = (id: string, number: number, head: string) => `viewed:${id}#${number}@${head}`;
export const replyKey = (repo: string, number: number) => `reply:${repo}#${number}`;

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
  useEffect(() => () => flush(key), [key]);
  const set = useCallback((next: T | ((old: T) => T)) => {
    const v = typeof next === "function" ? (next as (old: T) => T)(current.current) : next;
    if (JSON.stringify(v) === JSON.stringify(current.current)) return;
    current.current = v;
    setLocal(v);
    schedule(qc, key, v, JSON.stringify(v) === emptyJson);
  }, [qc, key, emptyJson]);
  /** Deletes the stored value now, for example after the draft was posted. */
  const clear = useCallback(() => {
    current.current = JSON.parse(emptyJson) as T;
    setLocal(current.current);
    return send(qc, key, null, true);
  }, [qc, key, emptyJson]);
  return { value, set, clear, loaded: q.isSuccess || q.isError, remote };
}
