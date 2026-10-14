// Client of the Shiki worker. Results are cached for the last files, checked against their text.

import { logToDaemon } from "@/lib/api";
import type { Theme } from "@/stores/theme";
import type { Tokens } from "./highlight.worker";
import { textLimits } from "./limits";

export type { Tokens };

let worker: Worker | null = null;
let next = 1;
const pending = new Map<number, { resolve: (t: Tokens | null) => void; timer: ReturnType<typeof setTimeout> }>();
const TIMEOUT_MS = 5_000;

function resetWorker() {
  if (worker) {
    worker.onmessage = null;
    worker.onerror = null;
    worker.onmessageerror = null;
    worker.terminate();
  }
  worker = null;
  for (const p of pending.values()) {
    clearTimeout(p.timer);
    p.resolve(null);
  }
  pending.clear();
}
const cache = new Map<string, { text: string; tokens: Tokens }>();

function getWorker() {
  if (!worker) {
    worker = new Worker(new URL("./highlight.worker.ts", import.meta.url), { type: "module" });
    worker.onmessage = (e: MessageEvent<{ id: number; tokens?: Tokens; error?: string }>) => {
      if (e.data.error) logToDaemon(`highlight: ${e.data.error}`, "warn");
      const p = pending.get(e.data.id);
      if (p) {
        clearTimeout(p.timer);
        pending.delete(e.data.id);
        p.resolve(e.data.tokens ?? null);
      }
    };
    // Failed or stalled workers are terminated; the next file gets a fresh worker.
    worker.onmessageerror = resetWorker;
    worker.onerror = (e) => {
      logToDaemon(`highlight worker: ${e.message || "failed to start"}`, "error");
      resetWorker();
    };
  }
  return worker;
}

/** Tokens already computed for this exact text, if any. */
export function cachedTokens(key: string, text: string, theme: Theme): Tokens | null {
  const hit = cache.get(`${theme}:${key}`);
  return hit && hit.text === text ? hit.tokens : null;
}

/** Colors for `text`, or null when the language has no grammar. */
export function highlight(key: string, text: string, lang: string, theme: Theme): Promise<Tokens | null> {
  if (!textLimits(text).colors) return Promise.resolve(null);
  const hit = cachedTokens(key, text, theme);
  if (hit) return Promise.resolve(hit);
  const id = next++;
  return new Promise((resolve) => {
    const timer = setTimeout(() => {
      logToDaemon("highlight worker timed out; using plain text", "warn");
      resetWorker();
    }, TIMEOUT_MS);
    pending.set(id, { timer, resolve: (tokens) => {
      if (tokens) {
        cache.delete(`${theme}:${key}`);
        cache.set(`${theme}:${key}`, { text, tokens });
        if (cache.size > 12) cache.delete(cache.keys().next().value!);
      }
      resolve(tokens);
    } });
    try {
      getWorker().postMessage({ id, code: text, lang, theme });
    } catch {
      resetWorker();
    }
  });
}
