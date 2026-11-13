// The Home client and the connection state. Every call to Home goes through `homeFetch`, which adds the device secret
// (`X-Launchpad-Device`) and, for a Space, the Hugging Face access token (`Authorization`) that passes the Hub's
// private-Space check. Chrome sets `Origin: chrome-extension://<id>` itself, which Home and the gateway require.

import type { Today } from "@/gen/Today";
import type { Workspace } from "@/gen/Workspace";
import { RequestError, type ConnectionState, type HomeInfo } from "../shared/messages";
import { renderBadge } from "./badge";
import { accessToken, clientId, homeStatus, refresh, signOut } from "./hf";
import { pushState } from "./ports";
import { local, removeLocal, removeSession, session, setLocal, setSession } from "./store";

let state: ConnectionState = { phase: "signed-out", home: null, user: null, client_id: null, workspaces: [], workspace: null, today: null, error: null };

export const current = () => state;

export function update(patch: Partial<ConnectionState>) {
  state = { ...state, ...patch };
  pushState(state);
  renderBadge(state);
}

/** Rebuilds the state from storage when the worker starts. */
export async function load() {
  const [home, device, user, workspace, id] = await Promise.all([local("home"), local("device"), local("hf.user"), local("workspace"), clientId()]);
  const phase = home && device ? "connecting" : user ? "signed-in" : "signed-out";
  update({ phase, home: home && device ? home : null, user: user ?? null, client_id: id, workspace: workspace ?? null });
  return !!(home && device);
}

async function deviceName() {
  const os = (await chrome.runtime.getPlatformInfo()).os;
  const names: Record<string, string> = { linux: "Linux", mac: "Mac", win: "Windows", cros: "ChromeOS", android: "Android" };
  return `Chrome on ${names[os] ?? os}`;
}

const REVOKED = /pair this browser|connect this browser/i;

/** A request to the connected Home. A refused device makes the phase `revoked`; a Space's 401 refreshes the token once. */
export async function homeFetch(path: string, init: { method?: string; body?: unknown; idempotency_key?: string; signal?: AbortSignal; headers?: Record<string, string> } = {}): Promise<Response> {
  const [home, device] = await Promise.all([local("home"), local("device")]);
  if (!home || !device) throw new RequestError("Not connected to a Launchpad Home", 0);
  const send = async () => {
    const headers: Record<string, string> = { "X-Launchpad-Device": device, ...init.headers };
    if (home.kind === "space") {
      const bearer = await accessToken();
      if (bearer) headers.Authorization = `Bearer ${bearer}`;
    }
    if (init.body !== undefined) headers["Content-Type"] = "application/json";
    if (init.idempotency_key) headers["Idempotency-Key"] = init.idempotency_key;
    return fetch(home.origin + path, { method: init.method ?? (init.body === undefined ? "GET" : "POST"), headers, body: init.body === undefined ? undefined : JSON.stringify(init.body), signal: init.signal, cache: "no-store" });
  };
  let response = await send();
  if (response.status === 401) {
    const text = await response.clone().text().catch(() => "");
    if (REVOKED.test(text)) {
      await removeLocal("device", "events.position");
      update({ phase: "revoked", error: "Home disconnected this browser. Connect it again." });
      throw new RequestError("Home disconnected this browser", 401);
    }
    if (home.kind === "space") {
      await refresh().catch(() => undefined);
      response = await send();
    }
  }
  return response;
}

/** A JSON request to Home; a refusal throws with Home's error text. */
export async function homeJson<T>(path: string, init: Parameters<typeof homeFetch>[1] = {}): Promise<T> {
  const response = await homeFetch(path, init);
  const data = await response.json().catch(() => null);
  if (!response.ok) throw new RequestError(data?.error ?? `Home answered ${response.status}`, response.status);
  return data as T;
}

// Workspace and Today (F4): one current workspace; the badge shows its Today count.

export async function loadWorkspaces() {
  const workspaces = await homeJson<Workspace[]>("/api/workspaces");
  const oldest = [...workspaces].sort((a, b) => a.created_at - b.created_at)[0];
  const chosen = workspaces.some((w) => w.id === state.workspace) ? state.workspace : oldest?.id ?? null;
  if (chosen !== state.workspace) await setLocal({ workspace: chosen ?? undefined });
  update({ workspaces, workspace: chosen });
}

export async function setWorkspace(id: string) {
  if (!state.workspaces.some((w) => w.id === id)) throw new RequestError("No such workspace");
  if (id === state.workspace) return;
  await setLocal({ workspace: id });
  update({ workspace: id, today: null });
  await loadToday();
}

/** An item without workspaces (shared) counts in every workspace, like the web app's `inWorkspace`. */
export async function loadToday() {
  const today = await homeJson<Today>("/api/today");
  const ws = state.workspace;
  update({ today: today.items.filter((i) => !i.workspaces.length || (!!ws && i.workspaces.includes(ws))).length });
}

let todayTimer: ReturnType<typeof setTimeout> | undefined;
/** Recomputes the count shortly after a `today` event (bursts of changes make one request). */
export function todayChanged() {
  clearTimeout(todayTimer);
  todayTimer = setTimeout(() => void loadToday().catch((e) => console.warn("today", e)), 300);
}

// Connecting.

const SPACE_ORIGIN = /^https:\/\/[a-z0-9-]+\.hf\.space$/;

/** After a new device secret: workspaces, Today, then the event stream (started by the caller through `onConnected`). */
let onConnected: () => void = () => {};
export const whenConnected = (fn: () => void) => { onConnected = fn; };

async function connected(home: HomeInfo, device: string) {
  await setLocal({ home, device });
  await removeLocal("events.position");
  update({ phase: "connecting", home, error: null, today: null });
  await loadWorkspaces();
  await loadToday();
  onConnected();
}

export async function connectSpace(space: string, origin: string): Promise<ConnectionState> {
  if (!SPACE_ORIGIN.test(origin)) throw new RequestError("Not a Hugging Face Space address");
  update({ phase: "connecting", error: null });
  const status = await homeStatus(origin);
  if (status.kind === "starting") { update({ phase: "starting" }); return state; }
  if (status.kind === "unreachable") { update({ phase: state.user ? "signed-in" : "signed-out", error: status.error }); throw new RequestError(status.error); }
  if (!status.complete) { update({ phase: "signed-in", error: "Finish Home's setup on its Space first." }); throw new RequestError("Finish Home's setup on its Space first"); }
  const bearer = await accessToken();
  const device_name = await deviceName();
  const response = await fetch(`${origin}/_home/extension/connect`, { method: "POST", headers: { Authorization: `Bearer ${bearer}`, "Content-Type": "application/json" }, body: JSON.stringify({ name: device_name }) });
  const data = await response.json().catch(() => null);
  if (!response.ok || !data?.device) {
    const error = data?.error ?? `Home answered ${response.status}`;
    update({ phase: "signed-in", error });
    throw new RequestError(error, response.status);
  }
  await connected({ kind: "space", origin, space, device_name }, data.device);
  return state;
}

/** The manual address: HTTPS, or HTTP on this computer. */
export function addressOrigin(input: string) {
  const url = new URL(input);
  const local = url.hostname === "127.0.0.1" || url.hostname === "localhost";
  if (url.username || url.password || url.search || url.hash || url.pathname !== "/" || !(url.protocol === "https:" || url.protocol === "http:" && local)) throw new RequestError("Use an HTTPS address, or HTTP on localhost");
  return url.origin;
}

async function post(origin: string, path: string, body: unknown) {
  const response = await fetch(origin + path, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });
  const data = await response.json().catch(() => null);
  if (!response.ok) throw new RequestError(data?.error ?? `Home answered ${response.status}`, response.status);
  return data;
}

/** Reverse pairing: Home shows nothing, the owner approves this code in Settings → Devices of a signed-in window. */
export async function startPairing(input: string) {
  const origin = addressOrigin(input);
  if (!(await chrome.permissions.contains({ origins: [`${origin}/*`] }))) throw new RequestError("Allow Launchpad to reach this address first");
  const device_name = await deviceName();
  const pair = await post(origin, "/auth/reverse/start", { name: device_name });
  await setSession({ pairing: { origin, code: pair.code, secret: pair.secret, expires_at: pair.expires_at * 1000, device_name } });
  update({ error: null });
  return { code: String(pair.code), expires_at: pair.expires_at * 1000 };
}

export async function pollPairing(): Promise<{ approved: boolean; state: ConnectionState }> {
  const pairing = await session("pairing");
  if (!pairing) throw new RequestError("No pairing in progress");
  if (Date.now() >= pairing.expires_at) { await removeSession("pairing"); throw new RequestError("The pairing code expired. Show a new code."); }
  try {
    const result = await post(pairing.origin, "/auth/reverse/poll", { code: pairing.code, secret: pairing.secret });
    if (!result?.ok || !result.device) return { approved: false, state };
    await removeSession("pairing");
    await connected({ kind: "address", origin: pairing.origin, device_name: pairing.device_name }, String(result.device));
    return { approved: true, state };
  } catch (e) {
    if (e instanceof RequestError && e.status === 429) return { approved: false, state };
    throw e;
  }
}

/** Removes this browser's device on Home (best effort), then every credential and choice except the client ID setting. */
export async function disconnect(stop: () => void) {
  stop();
  if (state.home && (await local("device"))) await homeFetch("/api/extension/disconnect", { method: "POST" }).catch((e) => console.warn("disconnect", e));
  await signOut();
  await removeLocal("home", "device", "workspace", "events.position");
  await removeSession("pairing");
  update({ phase: "signed-out", home: null, user: null, workspaces: [], workspace: null, today: null, error: null });
}

/** Signed in with Hugging Face, ready to find a Home (after sign-in, or after Home disconnected this browser). */
export async function signedIn() {
  update({ phase: "signed-in", user: (await local("hf.user")) ?? null, error: null });
}
