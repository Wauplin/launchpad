// The in-browser demo daemon: a route table over the in-memory demo world (`world/`), a fake event bus and fake
// sockets. In demo mode it answers every request the UI makes; the real daemon only receives `/api/onboarding`
// (tour progress). Handlers live in `routes/*.ts`, one file per API area, and register with `route()`.

import { toast } from "@/components/Toast";

export class DemoError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}

/** The answer to every action the demo does not simulate. */
export const NOT_IN_DEMO = "This action is not part of the demo.";
export const notInDemo = (): never => { throw new DemoError(400, NOT_IN_DEMO); };

export type DemoRequest = {
  method: string;
  path: string;
  query: URLSearchParams;
  /** Parsed JSON body (or the raw Blob for uploads). */
  body: any;
  /** `:name` segments of the route pattern, decoded. */
  params: Record<string, string>;
  headers: Headers;
};
type Handler = (req: DemoRequest) => unknown;
type Route = { method: string; pattern: string; regex: RegExp; keys: string[]; handler: Handler };

const routes: Route[] = [];

/**
 * Registers a handler. Patterns use `:name` for one segment and `*name` for the rest of the path, for example
 * `/api/sessions/:id/events/page` or `/api/items/*repo` (repositories contain `/`). `method` is GET, POST, …
 */
export function route(method: string, pattern: string, handler: Handler) {
  const keys: string[] = [];
  const source = pattern
    .replace(/[.+?^${}()|[\]\\]/g, "\\$&")
    .replace(/([:*])(\w+)/g, (_, kind: string, key: string) => { keys.push(key); return kind === ":" ? "([^/]+)" : "(.+)"; });
  routes.push({ method, pattern, regex: new RegExp(`^${source}$`), keys, handler });
}

/** Every registered `METHOD pattern`, for the coverage check (`pnpm test:tour`). */
export const registeredRoutes = () => routes.map((r) => `${r.method} ${r.pattern}`);

function match(method: string, path: string): { route: Route; params: Record<string, string> } | null {
  // Earlier registrations win, so specific patterns go before catch-all ones.
  for (const r of routes) {
    if (r.method !== method) continue;
    const m = r.regex.exec(path);
    if (!m) continue;
    const params: Record<string, string> = {};
    r.keys.forEach((k, i) => { params[k] = decodeURIComponent(m[i + 1]); });
    return { route: r, params };
  }
  return null;
}

const json = (status: number, data: unknown, headers: Record<string, string> = {}) =>
  new Response(data === undefined ? "null" : JSON.stringify(data), { status, headers: { "Content-Type": "application/json", ...headers } });

/** Requests the real daemon still answers in demo mode: the onboarding record only. */
const PASSTHROUGH = /^\/api\/onboarding$/;

/** Answers one `fetch` call of the UI. */
export async function demoFetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const request = input instanceof Request ? input : null;
  const url = new URL(request ? request.url : String(input), location.origin);
  const method = (init?.method ?? request?.method ?? "GET").toUpperCase();
  if (url.origin !== location.origin) return Promise.reject(new TypeError("Demo mode does not reach other sites."));
  if (PASSTHROUGH.test(url.pathname)) return window.__lpDemo!.realFetch(input, init);
  if (!url.pathname.startsWith("/api/")) {
    // Static assets (lazy chunks use import(), not fetch); anything else is refused.
    if (url.pathname.startsWith("/assets/") || url.pathname.startsWith("/src/") || url.pathname.startsWith("/node_modules/")) return window.__lpDemo!.realFetch(input, init);
    return json(404, { error: "Not part of the demo." });
  }
  const headers = new Headers(init?.headers ?? request?.headers);
  let body: unknown = init?.body ?? null;
  if (typeof body === "string") { try { body = JSON.parse(body); } catch { /* Raw text. */ } }
  const found = match(method, url.pathname);
  // Mutations answer like the real retry middleware so `lib/api.ts` accepts them.
  const idempotency: Record<string, string> = headers.get("Idempotency-Key") ? { "Idempotency-Status": "completed" } : {};
  if (!found) {
    const message = `Demo: no handler for ${method} ${url.pathname}`;
    if (import.meta.env.DEV) { console.error(message); toast(message); }
    return json(method === "GET" ? 404 : 400, { error: method === "GET" ? "Not part of the demo." : NOT_IN_DEMO }, idempotency);
  }
  await new Promise((r) => setTimeout(r, 30 + Math.random() * 60));
  try {
    const data = await found.route.handler({ method, path: url.pathname, query: url.searchParams, body, params: found.params, headers });
    if (data instanceof Response) return data;
    return json(200, data ?? null, idempotency);
  } catch (e) {
    if (e instanceof DemoError) return json(e.status, { error: e.message }, idempotency);
    console.error("demo handler failed", method, url.pathname, e);
    return json(500, { error: e instanceof Error ? e.message : String(e) }, idempotency);
  }
}

// ---------------------------------------------------------------- bus

type Socket = FakeSocket;
const busSockets = new Set<Socket>();
let busSeq = 0;
const BOOT = "demo";

/** Sends one event to every open bus socket, like `app.bus.publish` in the daemon. */
export function publish(topic: string, type: string, payload: unknown) {
  busSeq += 1;
  const ev = JSON.stringify({ bus: busSeq, seq: busSeq, topic, type, payload });
  for (const s of busSockets) s.deliver(ev);
}

// ---------------------------------------------------------------- sockets

type TerminalSocketHandler = (socket: FakeSocket, id: string) => void;
let terminalSocket: TerminalSocketHandler | null = null;
/** The terminal routes register how a terminal socket (`/api/terminals/{id}/ws`) is served. */
export function onTerminalSocket(fn: TerminalSocketHandler) { terminalSocket = fn; }

/** A WebSocket stand-in with the subset of the API the UI uses (`on*` handlers, send, close). */
export class FakeSocket {
  static CONNECTING = 0;
  static OPEN = 1;
  static CLOSING = 2;
  static CLOSED = 3;
  readonly CONNECTING = 0;
  readonly OPEN = 1;
  readonly CLOSING = 2;
  readonly CLOSED = 3;
  url: string;
  readyState = 0;
  binaryType: BinaryType = "blob";
  protocol = "";
  extensions = "";
  bufferedAmount = 0;
  onopen: ((ev: Event) => void) | null = null;
  onmessage: ((ev: MessageEvent) => void) | null = null;
  onclose: ((ev: CloseEvent) => void) | null = null;
  onerror: ((ev: Event) => void) | null = null;
  /** Messages from the UI (terminal input, resize, ack). */
  onSend: ((data: string | ArrayBufferLike | Blob | ArrayBufferView) => void) | null = null;
  onClosed: (() => void) | null = null;

  constructor(url: string | URL) {
    this.url = String(url);
    const path = new URL(this.url.replace(/^ws/, "http")).pathname;
    queueMicrotask(() => {
      if (this.readyState !== 0) return;
      this.readyState = 1;
      this.onopen?.(new Event("open"));
      if (path === "/ws") {
        busSockets.add(this);
        this.deliver(JSON.stringify({ type: "hello", bus: busSeq, seq: 0, topic: "", payload: null, build: "demo", boot: BOOT }));
        this.deliver(JSON.stringify({ type: "replayed", bus: busSeq, seq: 0, topic: "", payload: null }));
        return;
      }
      const terminal = path.match(/^\/api\/terminals\/([^/]+)\/ws$/);
      if (terminal && terminalSocket) terminalSocket(this, decodeURIComponent(terminal[1]));
      else this.close();
    });
  }

  /** Delivers one message to the UI (a string or binary output). */
  deliver(data: string | Uint8Array) {
    if (this.readyState !== 1) return;
    const payload = typeof data === "string" ? data : data.slice().buffer;
    this.onmessage?.(new MessageEvent("message", { data: payload }));
  }

  send(data: string | ArrayBufferLike | Blob | ArrayBufferView) {
    if (this.readyState !== 1) return;
    if (typeof data === "string") {
      try { if (JSON.parse(data).type === "ping") { this.deliver(JSON.stringify({ type: "pong", bus: busSeq, seq: 0, topic: "", payload: null })); return; } } catch { /* Not JSON. */ }
    }
    this.onSend?.(data);
  }

  close() {
    if (this.readyState >= 2) return;
    this.readyState = 3;
    busSockets.delete(this);
    this.onClosed?.();
    queueMicrotask(() => this.onclose?.(new CloseEvent("close")));
  }

  addEventListener() { /* The UI uses the on* handlers only. */ }
  removeEventListener() { /* The UI uses the on* handlers only. */ }
  dispatchEvent() { return false; }
}

// ---------------------------------------------------------------- clock

/**
 * The demo clock. World timestamps are offsets from the moment the demo starts, so "3 min ago" is always right.
 * `fastForward` moves it ahead (quota resets, Start later) without waiting.
 */
let skew = 0;
export const clock = {
  now: () => Date.now() + skew,
  /** Epoch ms of `minutes` ago (negative: in the future). */
  ago: (minutes: number) => Date.now() + skew - minutes * 60_000,
  fastForward(ms: number) { skew += ms; for (const fn of clockListeners) fn(); },
};
const clockListeners = new Set<() => void>();
export function onClock(fn: () => void) { clockListeners.add(fn); return () => clockListeners.delete(fn); }

/** Shown after a simulated GitHub action. */
export const githubDemo = (what = "Done") => toast(`Demo: ${what}. Nothing was sent to GitHub.`);
