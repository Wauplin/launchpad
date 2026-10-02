// Fetch wrapper for the daemon API. The session cookie is sent automatically (same origin).

export class ApiError extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(message);
  }
}

async function request<T>(method: string, path: string, body?: unknown, signal?: AbortSignal): Promise<T> {
  const res = await fetch(path, {
    method,
    signal,
    headers: body === undefined ? undefined : { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await res.text();
  const data = text ? JSON.parse(text) : null;
  if (!res.ok) {
    throw new ApiError(res.status, data?.error ?? res.statusText);
  }
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
