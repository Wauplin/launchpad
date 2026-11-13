// Bots, agents and suggestions for the panel and the GitHub layer: the workspace's bots (with its Ask target), bot
// messages, its sessions, a session's events, Stop, the feedback-ledger records, and the selection the context menu
// sends to the panel (P3). Every call goes to Home through `homeJson`, narrowed to the extension's workspace.

import type { Bot } from "@/gen/Bot";
import type { DiffSelection } from "@/gen/DiffSelection";
import type { LaunchSpec } from "@/gen/LaunchSpec";
import type { FileAttachment } from "@/gen/FileAttachment";
import type { GitHubRef } from "@/gen/GitHubRef";
import type { Project } from "@/gen/Project";
import type { Session } from "@/gen/Session";
import type { SessionEventPage } from "@/gen/SessionEventPage";
import { QUOTE_TOPIC, RequestError, type BotList, type LaunchWhenDone, type LedgerAction, type ProfileBrief, type Quote } from "../shared/messages";
import { current, homeJson } from "./home";
import { pushEvent } from "./ports";

const id = (v: unknown, what: string) => {
  if (typeof v !== "string" || !/^[A-Za-z0-9_-]{1,80}$/.test(v)) throw new RequestError(`Not a ${what} ID`);
  return v;
};
const workspace = () => {
  const ws = current().workspace;
  if (!ws) throw new RequestError("Choose a workspace in the Launchpad panel first");
  return ws;
};

// Folders change rarely: kept 30 s to narrow sessions to the workspace.
let folders: { at: number; list: Project[] } | null = null;
async function projects() {
  if (!folders || Date.now() - folders.at > 30_000) folders = { at: Date.now(), list: await homeJson<Project[]>("/api/projects") };
  return folders.list;
}

export async function bots(): Promise<BotList> {
  const ws = workspace();
  const all = (await homeJson<Bot[]>("/api/bots")).filter((b) => b.workspace_id === ws).sort((a, b) => a.created_at - b.created_at);
  // `askTarget` of web/src/lib/bots.ts: the oldest admin bot of the workspace, else its oldest bot.
  const ask = all.find((b) => b.settings.launchpad === "admin") ?? all[0] ?? null;
  return { bots: all.map((b) => ({ id: b.id, name: b.settings.name, slug: b.slug, workspace_id: b.workspace_id, paused: b.paused })), ask: ask?.id ?? null };
}

const refs = (v: unknown): GitHubRef[] => (Array.isArray(v) ? v.slice(0, 10) : []).filter((r): r is GitHubRef =>
  !!r && typeof r.url === "string" && r.url.startsWith("https://github.com/") && r.url.length < 500 && typeof r.repo === "string" && typeof r.kind === "string")
  .map((r) => ({ url: r.url, kind: r.kind, repo: r.repo, number: typeof r.number === "number" ? r.number : null, title: typeof r.title === "string" ? r.title.slice(0, 300) : null }));

const files = (v: unknown): FileAttachment[] => (Array.isArray(v) ? v.slice(0, 4) : []).filter((f): f is FileAttachment =>
  !!f && typeof f.name === "string" && typeof f.data_url === "string" && f.data_url.startsWith("data:") && f.data_url.length < 2_000_000)
  .map((f) => ({ name: f.name.slice(0, 200), data_url: f.data_url }));

/** A diff selection as a reference only: Home reads the lines (`lines` is never sent). */
function selection(v: unknown): DiffSelection | undefined {
  const s = v as Partial<DiffSelection> | null | undefined;
  if (!s || typeof s !== "object") return undefined;
  const n = (x: unknown) => (typeof x === "number" && Number.isInteger(x) && x > 0 && x < 1e9 ? x : null);
  const [number, start, end] = [n(s.number), n(s.start), n(s.end)];
  if (typeof s.repo !== "string" || !REPO.test(s.repo) || !number || !start || !end || end < start || end - start >= 400) throw new RequestError("Not a selection of a pull request's diff");
  if (typeof s.path !== "string" || !s.path || s.path.length > 1024 || s.path.startsWith("/") || s.path.split("/").some((x) => !x || x === "." || x === "..")) throw new RequestError("Not a file of the diff");
  if (s.side !== "LEFT" && s.side !== "RIGHT") throw new RequestError("Not a side of the diff");
  const head = typeof s.head === "string" && /^[0-9a-f]{7,64}$/.test(s.head) ? s.head : null;
  return { repo: s.repo, number, path: s.path, side: s.side, start, end, head };
}
const REPO = /^[A-Za-z0-9_.-]{1,100}\/[A-Za-z0-9_.-]{1,100}$/;

export async function botMessage(a: { bot: string; text: string; github_refs?: GitHubRef[]; attachments?: FileAttachment[]; new_session?: boolean; reason?: string; key: string; selection?: DiffSelection }): Promise<Session> {
  const bot = id(a.bot, "bot");
  const text = String(a.text ?? "").slice(0, 60_000);
  const sel = selection(a.selection);
  const body = { text, attachments: files(a.attachments), context: { github_refs: refs(a.github_refs), ...(sel ? { selection: sel } : {}) }, new_session: !!a.new_session, reason: a.reason ? String(a.reason).slice(0, 120) : null };
  return homeJson<Session>(`/api/bots/${bot}/message`, { method: "POST", body, idempotency_key: id(a.key, "message") });
}

/** Home's profiles (`GET /api/context/profiles`: never their environment), only what a picker shows. */
export async function profiles(): Promise<ProfileBrief[]> {
  type Brief = { id: string; name: string; harness: string; billing: string };
  const { profiles: list } = await homeJson<{ profiles: Brief[] }>("/api/context/profiles");
  return (list ?? []).map((p) => ({ id: p.id, name: p.name, harness: p.harness, subscription: p.billing === "subscription" }));
}

/** A one-time agent in a Git folder of the workspace, in its own worktree: read only, or ending in a Local PR. */
export async function launch(a: { folder: string; text: string; github_refs?: GitHubRef[]; when_done: LaunchWhenDone; profile: string; key: string }): Promise<Session> {
  const ws = workspace();
  const folder = id(a.folder, "folder");
  if (!(await projects()).some((p) => p.id === folder && p.workspace_id === ws)) throw new RequestError("This folder is not in the extension's workspace");
  if (a.when_done !== "read_only" && a.when_done !== "local_pr") throw new RequestError("Not a launch the GitHub layer makes");
  const profile = (await profiles()).find((p) => p.id === a.profile);
  if (!profile) throw new RequestError("No such profile on this Home");
  const text = String(a.text ?? "").trim().slice(0, 60_000);
  if (!text) throw new RequestError("The task is empty");
  const spec: Partial<LaunchSpec> = {
    text,
    harness: profile.harness as LaunchSpec["harness"],
    profile: profile.id,
    target: { kind: "project", project_id: folder },
    worktree: { base: null, when_done: a.when_done },
    github_refs: refs(a.github_refs),
    workspace_id: ws,
    permission_mode: "yolo",
  };
  return homeJson<Session>("/api/sessions", { method: "POST", body: spec, idempotency_key: id(a.key, "launch") });
}

/** The workspace of a session: its folder's, else the one its launch named. */
function sessionWorkspace(s: Session, list: Project[]) {
  const folder = s.project_id ? list.find((p) => p.id === s.project_id) : undefined;
  return folder ? folder.workspace_id : s.launch_spec.workspace_id;
}

export async function sessions(a: { bot?: string } | undefined): Promise<Session[]> {
  const ws = workspace();
  const [all, list] = await Promise.all([homeJson<Session[]>("/api/sessions"), projects()]);
  const mine = all.filter((s) => s.status !== "closed" && sessionWorkspace(s, list) === ws);
  if (a?.bot) return mine.filter((s) => s.launch_spec.bot?.id === a.bot).sort((x, y) => x.started_at - y.started_at);
  return mine.sort((x, y) => y.status_since - x.status_since);
}

export function events(a: { id: string; tail?: number }): Promise<SessionEventPage> {
  const tail = Math.max(1, Math.min(Number(a.tail) || 200, 500));
  return homeJson<SessionEventPage>(`/api/sessions/${id(a.id, "session")}/events/page?tail=${tail}`);
}

export async function stop(a: { id: string }) {
  await homeJson(`/api/sessions/${id(a.id, "session")}/interrupt`, { method: "POST", body: {} });
}

const LEDGER: readonly LedgerAction[] = ["shown", "inserted", "posted-by-hand", "dismiss-comment"];
export function ledger(a: { action: LedgerAction; id: string; part?: string; text?: string }) {
  if (!LEDGER.includes(a.action)) throw new RequestError("Not a ledger action");
  const part = typeof a.part === "string" ? a.part.slice(0, 600) : "body";
  const body: Record<string, unknown> = { part };
  if (typeof a.text === "string") body.text = a.text.slice(0, 262_144);
  return homeJson(`/api/suggestions/${id(a.id, "suggestion")}/${a.action}`, { method: "POST", body });
}

// P3: "Ask Launchpad about selection" (context menu). The selection waits here until the panel takes it.

export const QUOTE_MENU = "ask-selection";
let quote: Quote | null = null;

export function setupMenu() {
  chrome.contextMenus.removeAll(() => {
    chrome.contextMenus.create({ id: QUOTE_MENU, title: "Ask Launchpad about selection", contexts: ["selection"] });
  });
}

export function menuClicked(info: chrome.contextMenus.OnClickData, tab?: chrome.tabs.Tab) {
  if (info.menuItemId !== QUOTE_MENU || !info.selectionText) return;
  // The click is a user gesture: the panel may open now (it takes the quote when it starts).
  if (tab?.windowId !== undefined) void chrome.sidePanel.open({ windowId: tab.windowId }).catch((e) => console.warn("side panel", e));
  quote = { text: info.selectionText.slice(0, 20_000), url: tab?.url ?? info.pageUrl ?? "", title: tab?.title ?? "" };
  pushEvent({ bus: 0, seq: 0, topic: QUOTE_TOPIC, type: "quote", payload: quote });
}

export function takeQuote(): Quote | null {
  const q = quote;
  quote = null;
  return q;
}
