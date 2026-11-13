// Page contexts of the GitHub layer (F2): one `GET /api/ext/context` per page, cached 30 s per page and workspace and
// dropped when a bus event can change it (the worker then pushes the synthetic `ext.context` topic so the open pages
// ask again). Also the anchor health the GitHub layer reports, for Settings.

import type { ExtContext } from "@/gen/ExtContext";
import type { ExtRows } from "@/gen/ExtRows";
import { CONTEXT_TOPIC, type AnchorCheck, type AnchorHealth, type BusEvent } from "../shared/messages";
import { current, homeJson, setWorkspace } from "./home";
import { pushEvent } from "./ports";
import { local, setLocal } from "./store";

const TTL_MS = 30_000;
const MAX_ENTRIES = 60;

type Entry = { at: number; url: string; repo: string | null; data?: ExtContext; pending?: Promise<ExtContext> };
const cache = new Map<string, Entry>();

/** The page without its fragment: what Home's answer depends on. */
function pageUrl(url: string) {
  const u = new URL(url);
  if (u.origin !== "https://github.com") throw new Error("Not a github.com page");
  u.hash = "";
  return u.toString();
}

/** `owner/name` of a github.com URL, lowercase (only to match bus events). */
function repoOf(url: string) {
  const [owner, name] = new URL(url).pathname.split("/").filter(Boolean);
  return owner && name ? `${owner}/${name}`.toLowerCase() : null;
}

export async function pageContext(raw: string, fresh = false): Promise<ExtContext> {
  const url = pageUrl(raw);
  const ws = current().workspace;
  const key = `${ws ?? ""} ${url}`;
  const hit = cache.get(key);
  if (hit && !fresh) {
    if (hit.pending) return hit.pending;
    if (hit.data && Date.now() - hit.at < TTL_MS) return hit.data;
  }
  const entry: Entry = { at: Date.now(), url, repo: repoOf(url) };
  entry.pending = homeJson<ExtContext>(`/api/ext/context?url=${encodeURIComponent(url)}${ws ? `&workspace=${encodeURIComponent(ws)}` : ""}`)
    .then((data) => { entry.data = data; entry.at = Date.now(); entry.pending = undefined; return data; })
    .catch((e) => { if (cache.get(key) === entry) cache.delete(key); throw e; });
  cache.set(key, entry);
  while (cache.size > MAX_ENTRIES) cache.delete(cache.keys().next().value!);
  return entry.pending;
}

/** The newest cached context of any page of the same repository and workspace. */
export function peekContext(raw: string): ExtContext | null {
  const repo = repoOf(pageUrl(raw));
  const ws = current().workspace ?? "";
  let best: Entry | null = null;
  for (const [key, e] of cache) if (repo && e.repo === repo && e.data && key.startsWith(`${ws} `) && (!best || e.at > best.at)) best = e;
  return best?.data ?? null;
}

/** The context of the active tab's page: the extension follows the workspace of its repository's folder. */
export async function follow(data: ExtContext) {
  const ws = data.folder?.workspace_id;
  if (ws && ws !== current().workspace && current().workspaces.some((w) => w.id === ws)) await setWorkspace(ws);
}

export function pageRows(raw: string, items: string[]): Promise<ExtRows> {
  const url = pageUrl(raw);
  const ws = current().workspace;
  const list = items.filter((i) => /^([A-Za-z0-9]{1,40}|[A-Za-z0-9_.-]{1,100}\/[A-Za-z0-9_.-]{1,100}#[1-9]\d{0,9})$/.test(i)).slice(0, 100).map(encodeURIComponent).join(",");
  return homeJson<ExtRows>(`/api/ext/rows?url=${encodeURIComponent(url)}&items=${list}${ws ? `&workspace=${encodeURIComponent(ws)}` : ""}`);
}

function drop(which: (e: Entry) => boolean) {
  const urls: string[] = [];
  for (const [key, e] of cache) if (!e.pending && which(e)) { cache.delete(key); urls.push(e.url); }
  return urls;
}

function stale(urls: string[] | "*") {
  if (urls !== "*" && !urls.length) return;
  pushEvent({ bus: 0, seq: 0, topic: CONTEXT_TOPIC, type: "stale", payload: { urls } });
}

/** Topics whose events can change any page's context. */
const ALL = new Set(["today", "suggestions", "predrafts", "projects", "inbox", "bots", "workspaces", "worktrees", "home_git"]);

/** Called for every bus event of Home's stream. */
export function contextEvent(ev: BusEvent) {
  if (ALL.has(ev.topic) || (ev.topic === "sessions" && ev.type === "changed")) {
    drop(() => true);
    stale("*");
    return;
  }
  if (ev.topic !== "sessions" || ev.type !== "updated") return;
  // A session's status changed: the pages that list it, or whose repository its launch names.
  const s = ev.payload as { id?: string; launch_spec?: { github_refs?: { repo?: string }[] } } | null;
  const repos = new Set((s?.launch_spec?.github_refs ?? []).map((r) => r.repo?.toLowerCase()).filter(Boolean));
  stale(drop((e) => !!(e.data?.agents.some((a) => a.session_id === s?.id) || (e.repo && repos.has(e.repo)))));
}

/** Events were missed: every cached context may be wrong. */
export function contextResync() {
  cache.clear();
  stale("*");
}

// Anchor health: the last check of each anchor, kept across worker restarts.

export async function reportAnchors(checks: Record<string, Omit<AnchorCheck, "at">>) {
  const health = (await local("anchors")) ?? {};
  const at = Date.now();
  for (const [name, c] of Object.entries(checks)) {
    if (typeof c?.ok !== "boolean" || typeof c.label !== "string") continue;
    health[name.slice(0, 60)] = { label: c.label.slice(0, 80), ok: c.ok, strategy: typeof c.strategy === "string" ? c.strategy.slice(0, 20) : null, kind: String(c.kind).slice(0, 30), at };
  }
  await setLocal({ anchors: health });
}

export async function anchorHealth(): Promise<AnchorHealth> {
  return { anchors: (await local("anchors")) ?? {} };
}
