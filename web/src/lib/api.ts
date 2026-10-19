import { operationKey, retryProtected } from "./operations.ts";

// Fetch wrapper for the daemon API. The session cookie is sent automatically (same origin).

export class ApiError extends Error {
  status: number;
  operationCompleted: boolean;
  constructor(status: number, message: string, operationCompleted = false) {
    super(message);
    this.status = status;
    this.operationCompleted = operationCompleted;
  }
}

async function request<T>(method: string, path: string, body?: unknown, signal?: AbortSignal): Promise<T> {
  const operation = method === "POST" && retryProtected(path) ? await operationKey(path, body) : null;
  const res = await fetch(path, {
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
    if (res.status === 401) location.replace("/auth");
    throw new ApiError(res.status, data?.error ?? res.statusText, completed);
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
