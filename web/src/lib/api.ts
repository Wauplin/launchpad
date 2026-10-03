import { useRemote } from "./remote";
import { copyText } from "./clipboard";
import { toast } from "@/components/Toast";
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
  let copy = false;
  if (method === "POST" && useRemote.getState().enabled) {
    if (path === "/api/open") {
      const url = new URL((body as { url: string }).url);
      if (!["http:", "https:"].includes(url.protocol)) throw new ApiError(400, "Only HTTP and HTTPS URLs can be opened");
      window.open(url.href, "_blank", "noopener,noreferrer");
      return { ok: true } as T;
    } else if (path === "/api/open-editor") {
      body = { ...(body as object), copy_only: true };
      copy = true;
    } else if (/^\/api\/sessions\/[^/]+\/open-terminal$/.test(path)) {
      path = path.replace(/open-terminal$/, "terminal-command");
      copy = true;
    }
  }
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
  if (copy) { await copyText(data.command); toast("Command copied. Run it on the server."); }
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
