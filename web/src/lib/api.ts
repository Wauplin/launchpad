import { daemonUrl } from "./connection.ts";
import { signOut, storageAllowed } from "./clientStorage.ts";
import { operationKey, retryProtected } from "./operations.ts";

// Fetch wrapper for the daemon API. The session cookie is sent automatically (same origin).

export class ApiError extends Error {
  status: number;
  operationCompleted: boolean;
  /** The JSON body of the error answer (`{"error": …}` plus fields such as an oracle pre-check's `problems`). */
  body: Record<string, unknown> | null;
  constructor(status: number, message: string, operationCompleted = false, body: Record<string, unknown> | null = null) {
    super(message);
    this.status = status;
    this.operationCompleted = operationCompleted;
    this.body = body;
  }
}

/** A native client's WebView (the Android app's folio reader, `/folio-embed/`): the app answers the page's writes
 * itself, with its own credentials, so the page never holds a secret. `request` is answered later by
 * `window.__launchpadNative.resolve(id, status, bodyText)`; status 0: the app could not reach Home. */
type NativeBridge = { request: (id: string, method: string, path: string, body: string) => void; event: (json: string) => void };
declare global {
  interface Window {
    LaunchpadNative?: NativeBridge;
    __launchpadNative?: { resolve: (id: string, status: number, bodyText: string) => void; command: (json: string) => void };
  }
}
const nativePending = new Map<string, (answer: { status: number; text: string }) => void>();
let nativeNext = 1;

/** Installs `window.__launchpadNative` (once); `command` is the embed entry's handler of the app's commands. */
export function installNative(command: (json: string) => void) {
  window.__launchpadNative = {
    resolve: (id, status, bodyText) => { const done = nativePending.get(id); nativePending.delete(id); done?.({ status: Number(status) || 0, text: bodyText ?? "" }); },
    command,
  };
}

/** GET and HEAD stay plain fetches (the app intercepts them and adds its headers); every write goes through the bridge. */
async function nativeRequest<T>(native: NativeBridge, method: string, path: string, body?: unknown): Promise<T> {
  if (!window.__launchpadNative) installNative(() => {});
  const id = `r${nativeNext++}`;
  const answer = await new Promise<{ status: number; text: string }>((ok) => { nativePending.set(id, ok); native.request(id, method, daemonUrl(path), body === undefined ? "" : JSON.stringify(body)); });
  if (answer.status === 0) throw new TypeError(answer.text || "Failed to fetch");
  const okStatus = answer.status >= 200 && answer.status < 300;
  let data;
  try { data = answer.text ? JSON.parse(answer.text) : null; }
  catch (e) { if (okStatus) throw e; data = { error: answer.text }; }
  if (!okStatus) throw new ApiError(answer.status, data?.error ?? `HTTP ${answer.status}`, false, data && typeof data === "object" ? data as Record<string, unknown> : null);
  return data as T;
}

async function request<T>(method: string, path: string, body?: unknown, signal?: AbortSignal): Promise<T> {
  const native = typeof window === "undefined" ? undefined : window.LaunchpadNative;
  if (native && method !== "GET" && method !== "HEAD") return nativeRequest<T>(native, method, path, body);
  if (!storageAllowed()) throw new ApiError(401, "Signed out");
  const operation = method === "POST" && retryProtected(path) ? await operationKey(path, body) : null;
  const res = await fetch(daemonUrl(path), {
    credentials: "include",
    method,
    signal,
    headers: { ...(body === undefined ? {} : { "Content-Type": "application/json" }), ...(operation ? { "Idempotency-Key": operation.id } : {}) },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await res.text();
  let data;
  try { data = text ? JSON.parse(text) : null; }
  catch (e) { if (res.ok) throw e; data = { error: text }; }
  const completed = res.headers.get("Idempotency-Status") === "completed";
  if (completed) operation?.delivered();
  if (!res.ok) {
    // In a native client's WebView the app owns the sign-in: the page keeps its storage.
    if (res.status === 401 && !native) await signOut();
    throw new ApiError(res.status, data?.error ?? res.statusText, completed, data && typeof data === "object" ? data as Record<string, unknown> : null);
  }
  // Older daemons do not advertise durable acceptance; never silently disable retry protection.
  if (operation && !completed) throw new Error("The daemon needs an update before this action can be safely retried. Inspect the destination before trying again.");
  return data as T;
}

export const api = {
  /** `signal` cancels the request (TanStack Query passes one when a newer query replaces it). */
  get: <T>(path: string, signal?: AbortSignal) => request<T>("GET", path, undefined, signal),
  post: <T>(path: string, body?: unknown) => request<T>("POST", path, body ?? {}),
  put: <T>(path: string, body: unknown) => request<T>("PUT", path, body),
  patch: <T>(path: string, body: unknown) => request<T>("PATCH", path, body),
  delete: <T>(path: string) => request<T>("DELETE", path),
};

/** Writes a line to the daemon log (journalctl --user -u launchpad). */
export function logToDaemon(message: string, level: "info" | "warn" | "error" = "info") {
  void request("POST", "/api/log", { level, message }).catch(() => {});
}
