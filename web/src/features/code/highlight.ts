// Client of the Shiki worker. Results are cached for the last files, checked against their text.

import { logToDaemon } from "@/lib/api";
import type { Theme } from "@/stores/theme";
import type { Tokens } from "./highlight.worker";

export type { Tokens };

let worker: Worker | null = null;
let next = 1;
const pending = new Map<number, (t: Tokens | null) => void>();
const cache = new Map<string, { text: string; tokens: Tokens }>();

function getWorker() {
  if (!worker) {
    worker = new Worker(new URL("./highlight.worker.ts", import.meta.url), { type: "module" });
    worker.onmessage = (e: MessageEvent<{ id: number; tokens?: Tokens; error?: string }>) => {
      if (e.data.error) logToDaemon(`highlight: ${e.data.error}`, "warn");
      pending.get(e.data.id)?.(e.data.tokens ?? null);
      pending.delete(e.data.id);
    };
    // A worker that fails to load: plain text from now on.
    worker.onerror = (e) => {
      logToDaemon(`highlight worker: ${e.message || "failed to start"}`, "error");
      pending.forEach((resolve) => resolve(null));
      pending.clear();
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
  const hit = cachedTokens(key, text, theme);
  if (hit) return Promise.resolve(hit);
  const id = next++;
  return new Promise((resolve) => {
    pending.set(id, (tokens) => {
      if (tokens) {
        cache.delete(`${theme}:${key}`);
        cache.set(`${theme}:${key}`, { text, tokens });
        if (cache.size > 12) cache.delete(cache.keys().next().value!);
      }
      resolve(tokens);
    });
    getWorker().postMessage({ id, code: text, lang, theme });
  });
}
