// Sign in with Hugging Face (OAuth authorization code with PKCE, public client) and discovery of the Launchpad Space.
// The access token goes to chrome.storage.session and is refreshed before it expires; the refresh token stays in
// chrome.storage.local. Only the worker reads them.

import type { FoundHome } from "../shared/messages";
import { local, removeLocal, removeSession, session, setLocal, setSession } from "./store";

const HUB = "https://huggingface.co";
const SCOPES = "openid profile read-repos";
/** Refresh this long before the access token expires. */
const MARGIN_MS = 5 * 60_000;
export const REFRESH_ALARM = "hf-refresh";

export async function clientId(): Promise<{ value: string; source: "build" | "settings" } | null> {
  const own = (await local("hf.client_id"))?.trim();
  if (own) return { value: own, source: "settings" };
  const built = (import.meta.env.VITE_HF_CLIENT_ID ?? "").trim();
  return built ? { value: built, source: "build" } : null;
}

const base64url = (bytes: Uint8Array) => btoa(String.fromCharCode(...bytes)).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
const random = (n: number) => base64url(crypto.getRandomValues(new Uint8Array(n)));

type TokenAnswer = { access_token: string; expires_in?: number; refresh_token?: string };

async function token(form: Record<string, string>): Promise<TokenAnswer> {
  const response = await fetch(`${HUB}/oauth/token`, { method: "POST", headers: { "Content-Type": "application/x-www-form-urlencoded" }, body: new URLSearchParams(form) });
  const data = await response.json().catch(() => ({}));
  if (!response.ok || !data.access_token) throw new Error(`Hugging Face refused the sign-in: ${data.error_description ?? data.error ?? response.status}`);
  return data as TokenAnswer;
}

async function keep(answer: TokenAnswer) {
  const expires_at = Date.now() + (answer.expires_in ?? 3600) * 1000;
  await setSession({ "hf.access": { token: answer.access_token, expires_at } });
  if (answer.refresh_token) await setLocal({ "hf.refresh": answer.refresh_token });
  await chrome.alarms.create(REFRESH_ALARM, { when: Math.max(Date.now() + 30_000, expires_at - MARGIN_MS) });
}

/** The authorization flow in a Chrome sign-in window; `interactive: false` renews silently when Hugging Face allows. */
async function authorize(interactive: boolean): Promise<void> {
  const id = await clientId();
  if (!id) throw new Error("Sign in with Hugging Face is not configured yet");
  const verifier = random(48);
  const challenge = base64url(new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(verifier))));
  const state = random(16);
  const redirect = chrome.identity.getRedirectURL();
  const url = `${HUB}/oauth/authorize?${new URLSearchParams({ client_id: id.value, redirect_uri: redirect, response_type: "code", scope: SCOPES, state, code_challenge: challenge, code_challenge_method: "S256" })}`;
  const back = await chrome.identity.launchWebAuthFlow({ url, interactive });
  if (!back) throw new Error("The sign-in window closed");
  const params = new URL(back).searchParams;
  if (params.get("error")) throw new Error(params.get("error_description") ?? params.get("error")!);
  if (params.get("state") !== state) throw new Error("The sign-in answer did not match this request");
  const code = params.get("code");
  if (!code) throw new Error("Hugging Face returned no authorization code");
  await keep(await token({ grant_type: "authorization_code", code, redirect_uri: redirect, client_id: id.value, code_verifier: verifier }));
}

export async function signIn() {
  await authorize(true);
  const me = await whoami();
  await setLocal({ "hf.user": { name: me.name, avatar: me.avatarUrl } });
}

export async function signOut() {
  await removeSession("hf.access");
  await removeLocal("hf.refresh", "hf.user");
  await chrome.alarms.clear(REFRESH_ALARM);
}

let refreshing: Promise<void> | null = null;

/** Renews the access token with the refresh token, else silently through the authorization page. */
export function refresh(): Promise<void> {
  refreshing ??= (async () => {
    const id = await clientId();
    const stored = await local("hf.refresh");
    try {
      if (!id || !stored) throw new Error("no refresh token");
      await keep(await token({ grant_type: "refresh_token", refresh_token: stored, client_id: id.value }));
    } catch {
      await authorize(false);
    }
  })().finally(() => { refreshing = null; });
  return refreshing;
}

/** A valid access token, refreshed when it expires soon; null when not signed in with Hugging Face. */
export async function accessToken(): Promise<string | null> {
  const current = await session("hf.access");
  if (current && current.expires_at - MARGIN_MS > Date.now()) return current.token;
  if (!current && !(await local("hf.refresh")) && !(await local("hf.user"))) return null;
  await refresh();
  return (await session("hf.access"))?.token ?? null;
}

async function hub<T>(path: string): Promise<T> {
  const bearer = await accessToken();
  if (!bearer) throw new Error("Sign in with Hugging Face first");
  const response = await fetch(`${HUB}${path}`, { headers: { Authorization: `Bearer ${bearer}` } });
  if (!response.ok) throw new Error(`Hugging Face answered ${response.status} for ${path.split("?")[0]}`);
  return response.json() as Promise<T>;
}

type Whoami = { name: string; avatarUrl?: string; orgs?: { name: string }[] };
const whoami = () => hub<Whoami>("/api/whoami-v2");

type SpaceInfo = { id: string; host?: string; subdomain?: string; runtime?: { stage?: string }; cardData?: { title?: string; tags?: string[] }; tags?: string[] };
const TAG = "launchpad-home";
const TITLE = "Launchpad Home";

/** The Launchpad Spaces of the account, then of its organizations, in that order. */
export async function discover(): Promise<FoundHome[]> {
  const me = await whoami();
  await setLocal({ "hf.user": { name: me.name, avatar: me.avatarUrl } });
  const authors = [me.name, ...(me.orgs ?? []).map((o) => o.name)];
  const found: FoundHome[] = [];
  for (const author of authors) {
    const ids = new Set<string>();
    const [tagged, named] = await Promise.all([
      hub<SpaceInfo[]>(`/api/spaces?${new URLSearchParams({ author, filter: TAG })}`).catch(() => []),
      // Spaces deployed before the tag: the card title is "Launchpad Home".
      hub<SpaceInfo[]>(`/api/spaces?${new URLSearchParams({ author, search: "launchpad", full: "true" })}`).catch(() => []),
    ]);
    tagged.forEach((s) => ids.add(s.id));
    named.filter((s) => s.cardData?.title === TITLE || s.tags?.includes(TAG)).forEach((s) => ids.add(s.id));
    for (const id of ids) {
      const info = await hub<SpaceInfo>(`/api/spaces/${id}`).catch(() => null);
      const origin = info?.host ?? (info?.subdomain ? `https://${info.subdomain}.hf.space` : null);
      if (!info || !origin) continue;
      const stage = info.runtime?.stage ?? null;
      // Only running Spaces are asked: a request would wake a sleeping one.
      const answers = stage === "RUNNING" && (await homeStatus(origin)).kind === "home";
      found.push({ space: id, origin, stage, answers });
    }
  }
  return found;
}

export type HomeStatus = { kind: "home"; complete: boolean; space: string } | { kind: "starting" } | { kind: "unreachable"; error: string };

/** `/_home/status` of a Space: Launchpad's answer, the Hub's page while the Space starts, or no answer. */
export async function homeStatus(origin: string): Promise<HomeStatus> {
  try {
    const bearer = await accessToken();
    const response = await fetch(`${origin}/_home/status`, { headers: bearer ? { Authorization: `Bearer ${bearer}` } : {}, cache: "no-store" });
    const data = await response.json().catch(() => null);
    if (response.ok && data?.launchpad === true) return { kind: "home", complete: !!data.complete, space: String(data.space ?? "") };
    if (response.status === 401 || response.status === 403 || response.status === 404) return { kind: "unreachable", error: "Hugging Face refused access to this Space" };
    return { kind: "starting" };
  } catch (e) {
    return { kind: "unreachable", error: String(e instanceof Error ? e.message : e) };
  }
}
