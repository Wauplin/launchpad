import { sha256 } from "./operationHash.ts";

// Expiration requires owner review; never silently turn an uncertain send into a duplicate.
export const OPERATION_RETRY_MS = 24 * 60 * 60 * 1000;
type LedgerStorage = Pick<Storage, "getItem" | "setItem" | "removeItem">;
type Entry = { id: string; createdAt: number };

async function ledgerKey(path: string, body: unknown) {
  const bytes = new TextEncoder().encode(`${path}\n${JSON.stringify(body)}`);
  const hash = crypto.subtle
    ? Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)), (b) => b.toString(16).padStart(2, "0")).join("")
    : sha256(bytes);
  return `launchpad.operation.${hash}`;
}

function uuid() {
  if (crypto.randomUUID) return crypto.randomUUID();
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

// Keep the same operation ID after a lost response, including across page reloads.
// Store only a fingerprint, ID and creation time; no prompts, images or credentials.
export async function operationKey(path: string, body: unknown, storage: LedgerStorage = localStorage) {
  const key = await ledgerKey(path, body);
  const existing = storage.getItem(key);
  const now = Date.now();
  let entry: Entry;
  if (existing !== null) {
    // Legacy IDs have no known age. Treat them like expired entries, retaining
    // them until the owner explicitly abandons the uncertain operation.
    try { entry = JSON.parse(existing) as Entry; }
    catch { throw new Error("This saved attempt has no retry date. Inspect Agents or the trace, then choose start a new attempt."); }
    if (!entry || typeof entry.id !== "string" || !Number.isFinite(entry.createdAt)
      || entry.createdAt > now || now - entry.createdAt >= OPERATION_RETRY_MS) {
      throw new Error("This saved attempt is too old to retry automatically. Inspect Agents or the trace, then choose start a new attempt.");
    }
  } else {
    entry = { id: uuid(), createdAt: now };
    // If durable storage fails, stop before sending rather than risk a duplicate after reload.
    storage.setItem(key, JSON.stringify(entry));
  }
  const saved = JSON.stringify(entry);
  return { id: entry.id, delivered: () => { if (storage.getItem(key) === saved) storage.removeItem(key); } };
}

export function retryProtected(path: string): boolean {
  return path === "/api/sessions" || /^\/api\/sessions\/[^/]+\/input$/.test(path);
}

/** Owner explicitly starts another attempt after inspecting an uncertain result. */
export async function abandonOperation(path: string, body: unknown, storage: LedgerStorage = localStorage) {
  storage.removeItem(await ledgerKey(path, body));
}
