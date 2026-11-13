// The runtime-message protocol between the service worker (the only part that talks to Home) and its clients: the side
// panel and the content scripts. Clients never see a token or the device secret: they ask the worker, which adds the
// credentials. Add a request by adding an entry to `Requests` and a case to the worker's router (background/router.ts).

import type { BotThread } from "@/gen/BotThread";
import type { DiffSelection } from "@/gen/DiffSelection";
import type { ExtContext } from "@/gen/ExtContext";
import type { ExtNotifications } from "@/gen/ExtNotifications";
import type { ExtReviewChoice } from "@/gen/ExtReviewChoice";
import type { ExtRows } from "@/gen/ExtRows";
import type { ObservedResult } from "@/gen/ObservedResult";
import type { Predraft } from "@/gen/Predraft";
import type { Project } from "@/gen/Project";
import type { FileAttachment } from "@/gen/FileAttachment";
import type { GitHubRef } from "@/gen/GitHubRef";
import type { Session } from "@/gen/Session";
import type { SessionEventPage } from "@/gen/SessionEventPage";
import type { Workspace } from "@/gen/Workspace";

/** Where the extension is connected. Never holds a credential. */
export type HomeInfo = {
  kind: "space" | "address";
  /** `https://<subdomain>.hf.space` or the manual address. */
  origin: string;
  /** `owner/name` of the Space. */
  space?: string;
  /** The name of this browser in Home's Settings → Devices. */
  device_name: string;
};

export type Phase =
  | "signed-out" // nothing stored
  | "signed-in" // Hugging Face sign-in done, no Home chosen yet
  | "connecting"
  | "starting" // the Space wakes up: /_home/status does not answer yet
  | "online" // the event stream is open
  | "offline" // Home did not answer: retrying
  | "revoked"; // Home refused this browser's device (disconnected in Settings → Devices)

export type ConnectionState = {
  phase: Phase;
  home: HomeInfo | null;
  /** The Hugging Face account, after Sign in with Hugging Face. */
  user: { name: string; avatar?: string } | null;
  /** The OAuth client ID in use, and where it comes from (null: sign-in is not configured). */
  client_id: { value: string; source: "build" | "settings" } | null;
  workspaces: Workspace[];
  /** The extension's current workspace. */
  workspace: string | null;
  /** Today's count of the current workspace (null before the first answer). */
  today: number | null;
  /** Settings → GitHub: fill GitHub's comment box with an agent's suggested comment when an item page opens (on by
   * default). Posting stays GitHub's own button. */
  prefill: boolean;
  error: string | null;
};

/** A Launchpad Space found by discovery. */
export type FoundHome = {
  space: string;
  origin: string;
  /** The Hub's runtime stage (RUNNING, SLEEPING, BUILDING…), when known. */
  stage: string | null;
  /** /_home/status answered as Launchpad (only checked for running Spaces of the account itself). */
  answers: boolean;
  /** A Space of the signed-in account (not of an organization): only these are asked before the owner chooses one,
   * since every request to a Space carries the owner's Hugging Face token. */
  own: boolean;
};

/** One anchor of the GitHub layer (content/github/anchors.ts) as last checked on a page where it is expected. */
export type AnchorCheck = {
  /** Its name in Settings ("Repository name"). */
  label: string;
  ok: boolean;
  /** The strategy that matched (`data`, `aria`, `text`…), null when none did. */
  strategy: string | null;
  /** The page kind it was checked on. */
  kind: string;
  at: number;
};
export type AnchorHealth = { anchors: Record<string, AnchorCheck> };

/** The worker pushes this synthetic bus topic when page contexts it cached went stale: `payload.urls` lists the pages
 * (or `"*"` for all). The GitHub layer subscribes to it and asks for its page's context again. */
export const CONTEXT_TOPIC = "ext.context";

/** The worker pushes this synthetic topic when the context menu's "Ask Launchpad about selection" ran: `payload` is a
 * `Quote`. The panel adds it to its message box (never sends it by itself). */
export const QUOTE_TOPIC = "ext.quote";
export type Quote = { text: string; url: string; title: string };

/** A bot of the extension's workspace, for the pickers. */
export type BotBrief = { id: string; name: string; slug: string; workspace_id: string; paused: boolean };
export type BotList = { bots: BotBrief[]; /** The workspace's Ask target (`askTarget`: the oldest admin bot, else the oldest). */ ask: string | null };

/** The feedback-ledger actions the GitHub layer records (`suggestion.<action>` of Home's action list). */
export type LedgerAction = "shown" | "inserted" | "posted-by-hand" | "dismiss-comment";

/** Commands the panel sends to the GitHub layer of a tab (`chrome.tabs.sendMessage`): the same buttons as the page. */
export type TabCommand = { type: "insert"; suggestion: string };

/** Done, mark as read or Unsubscribe, clicked on github.com's notifications. */
export type ObservedAction = "done" | "read" | "unsubscribe";
/** One thread of Done all handled: marked Done, or why not. */
export type DoneResult = { id: string; error: string | null };
/** `GET /api/predrafts/{id}`: what the Publish dialog needs (the commit shown is the one published). */
export type LocalPrDetail = { predraft: Predraft; status: string | null; head?: string; default_remote_branch?: string; base_ahead: number; project_missing?: boolean; change_set?: { id: string; title: string; published_url: string | null; merged: boolean }[] };

/** A profile of Home (an agent CLI with its login), for the launch popovers; never its environment. */
export type ProfileBrief = { id: string; name: string; harness: string; subscription: boolean };
/** What a launch of the GitHub layer does when its agent is done (`WhenDone`). */
export type LaunchWhenDone = "read_only" | "local_pr";
/** Push to this PR's answer. */
export type PushResult = { ok: boolean; url: string; branch: string; head: string };

type Req<A, R> = { args: A; result: R };

/** Every request a client may send. `home.get` and `home.post` take Home paths under `/api/`. */
export type Requests = {
  state: Req<void, ConnectionState>;
  // Extension pages only (refused from content scripts):
  "signin": Req<void, ConnectionState>;
  "discover": Req<void, FoundHome[]>;
  /** A Space of `discover` (origin `https://<subdomain>.hf.space`); answers with phase `starting` while it wakes up:
   * ask again a few seconds later. */
  "connect.space": Req<{ space: string; origin: string }, ConnectionState>;
  "connect.address.start": Req<{ origin: string }, { code: string; expires_at: number }>;
  "connect.address.poll": Req<void, { approved: boolean; state: ConnectionState }>;
  "disconnect": Req<void, ConnectionState>;
  "settings.client-id": Req<{ client_id: string | null }, ConnectionState>;
  "settings.prefill": Req<{ on: boolean }, ConnectionState>;
  "workspace.set": Req<{ id: string }, ConnectionState>;
  /** The GitHub layer follows the repository's workspace (the worker also does it itself on `context`). */
  "workspace.follow": Req<{ id: string }, ConnectionState>;
  /** The GitHub layer's anchor health, for Settings ("11 of 12 work"). */
  "anchors.health": Req<void, AnchorHealth>;
  // Every client:
  /** Home's bundle about a github.com page (`GET /api/ext/context`), cached 30 s per page and workspace, dropped on
   * relevant bus events. `fresh` skips the cache. From the active tab, a page whose repository has a folder makes the
   * extension follow that folder's workspace. */
  "context": Req<{ url: string; fresh?: boolean }, ExtContext>;
  /** The newest context the worker holds for any page of the same repository (null: none), without calling Home: what
   * the GitHub layer shows at once while `context` loads. */
  "context.peek": Req<{ url: string }, ExtContext | null>;
  /** Row states of a list page (`GET /api/ext/rows`): numbers on Pull requests and Issues, `owner/repo#number` (or
   * thread IDs) on Notifications. */
  "rows": Req<{ url: string; items: string[] }, ExtRows>;
  /** Opens a page of Home (a path such as `/projects/<id>`) in a new tab. */
  "open": Req<{ path: string }, void>;
  /** The GitHub layer's anchor checks of one page. */
  "anchors.report": Req<{ checks: Record<string, Omit<AnchorCheck, "at">> }, void>;
  /** The bots of the extension's workspace and its Ask target. */
  "bots": Req<void, BotList>;
  /** A message to a bot's thread (`bot.message`); returns its session. `key` is the idempotency key (one per Send).
   * `github_refs` names the GitHub item or comment by reference; the bot reads the content itself. */
  "bot.message": Req<{ bot: string; text: string; github_refs?: GitHubRef[]; attachments?: FileAttachment[]; new_session?: boolean; reason?: string; key: string; /** Lines of a pull request's diff (Ask about these lines): a reference, Home reads the lines. */ selection?: DiffSelection; /** The bot's thread (the panel's picker); none: the thread of the one GitHub item named (several: a new one). */ thread?: string; /** A new thread (New conversation). */ new_thread?: boolean }, Session>;
  /** A bot's threads (main first, then the newest). */
  "bot.threads": Req<{ bot: string }, BotThread[]>;
  /** Home's profiles (id, name, CLI), for a launch's picker. */
  "profiles": Req<void, ProfileBrief[]>;
  /** A one-time agent in a folder of the workspace (`session.launch`): read only (Investigate, Check with an agent) or
   * ending in a Local PR (Fix in a Local PR). `key` is the idempotency key (one per Launch click). */
  "launch": Req<{ folder: string; text: string; github_refs?: GitHubRef[]; when_done: LaunchWhenDone; profile: string; key: string }, Session>;
  /** Push to this PR (`branch.push-to-pr`, a GitHub write): only from the confirm button of the extension's own
   * dialog, with the branch commit the dialog showed; Home reads the pull request again and pushes only a
   * fast-forward. */
  "branch.push-to-pr": Req<{ repo: string; number: number; branch: string; head: string }, PushResult>;
  /** The sessions of the extension's workspace that are not archived (light records), newest first; `bot` narrows to a
   * bot's thread (oldest first). */
  "sessions": Req<{ bot?: string }, Session[]>;
  /** The newest events of a session (`events/page?tail=`). */
  "session.events": Req<{ id: string; tail?: number }, SessionEventPage>;
  /** Deletes an agent's private note about a GitHub item (`note.delete`). */
  "note.delete": Req<{ id: string }, void>;
  /** Stops a session's turn (`session.stop`). */
  "session.stop": Req<{ id: string }, void>;
  /** A feedback-ledger record of a suggestion (`suggestion.shown`, `.inserted`, `.posted-by-hand`, `.dismiss-comment`). */
  "suggestion.ledger": Req<{ action: LedgerAction; id: string; part?: string; text?: string }, unknown>;
  /** The notifications strip (`GET /api/ext/notifications`): Today's counts of the workspace and the threads a bot
   * already handled. */
  "notifications": Req<void, ExtNotifications>;
  /** Done, read or Unsubscribe clicked on github.com (`inbox.observed`): recorded in Launchpad's Inbox, never sent to
   * GitHub again. `items`: `owner/repo#number` or thread IDs. */
  "inbox.observed": Req<{ items: string[]; action: ObservedAction }, ObservedResult[]>;
  /** Done all handled (`inbox.done-handled`, a GitHub write): only from the confirm button that lists the threads. */
  "inbox.done-handled": Req<{ ids: string[] }, DoneResult[]>;
  /** Remembers the bot and preset of Review with agent on a repository (`github.review-choice`). */
  "review-choice": Req<{ repo: string; bot?: string | null; preset?: string | null }, ExtReviewChoice>;
  /** Add to Launchpad: Clone from GitHub into a workspace (`folder.clone`). */
  "folder.clone": Req<{ repo: string; workspace_id: string }, Project>;
  /** A Local PR as its Launchpad page shows it (head, default remote branch, change set). */
  "localpr.detail": Req<{ id: string }, LocalPrDetail>;
  /** Publish (`local-pr.publish`, a GitHub write): only from the confirm button of the Publish dialog, with the commit
   * the dialog showed. */
  "localpr.publish": Req<{ id: string; remote_branch: string; draft: boolean; head: string; fork: boolean }, unknown>;
  /** The selection the context menu sent, once (the panel takes it when it opens). There is no request that posts to
   * GitHub: Launchpad's Post stays a click in Launchpad. */
  "quote.take": Req<void, Quote | null>;
};
export type Kind = keyof Requests;
export type Args<K extends Kind> = Requests[K]["args"];
export type Result<K extends Kind> = Requests[K]["result"];

export type Message<K extends Kind = Kind> = { kind: K; args: Args<K> };
export type Answer<K extends Kind = Kind> = { ok: true; data: Result<K> } | { ok: false; error: string; status?: number };

/** Requests only extension pages (the panel) may send. */
export const PAGE_ONLY: readonly Kind[] = ["signin", "discover", "connect.space", "connect.address.start", "connect.address.poll", "disconnect", "settings.client-id", "settings.prefill", "workspace.set", "anchors.health", "quote.take"];

/** An error of a request, with Home's HTTP status when there was one. */
export class RequestError extends Error {
  constructor(message: string, readonly status?: number) { super(message); }
}

/** Sends a request to the worker. */
export async function request<K extends Kind>(kind: K, ...args: Args<K> extends void ? [] : [Args<K>]): Promise<Result<K>> {
  const answer = await chrome.runtime.sendMessage<Message<K>, Answer<K> | undefined>({ kind, args: args[0] as Args<K> });
  if (!answer) throw new RequestError("Launchpad's background worker did not answer");
  if (!answer.ok) throw new RequestError(answer.error, answer.status);
  return answer.data;
}

// Subscriptions: a long-lived port named PORT. The client sends a `Subscribe`; the worker sends the current state at
// once, then every state change and the bus events of the topics asked for.

export const PORT = "launchpad";

/** A bus event of Home, as `/ws` and `/api/events` send it. */
export type BusEvent = { bus: number; seq: number; topic: string; type: string; payload: unknown };

export type Subscribe = { type: "subscribe"; topics: string[] | "*" };
export type Push =
  | { type: "state"; state: ConnectionState }
  | { type: "event"; event: BusEvent }
  /** Events were missed (Home restarted or the client lagged): reload what you show. */
  | { type: "resync" };

/** Subscribes to state changes and bus events. Returns the unsubscribe function; reconnects when the worker restarts. */
export function subscribe(topics: string[] | "*", on: { state?: (s: ConnectionState) => void; event?: (e: BusEvent) => void; resync?: () => void }): () => void {
  let port: chrome.runtime.Port | null = null;
  let closed = false;
  const open = () => {
    port = chrome.runtime.connect({ name: PORT });
    port.onMessage.addListener((m: Push) => {
      if (m.type === "state") on.state?.(m.state);
      else if (m.type === "event") on.event?.(m.event);
      else on.resync?.();
    });
    port.onDisconnect.addListener(() => { port = null; if (!closed) setTimeout(open, 500); });
    port.postMessage({ type: "subscribe", topics } satisfies Subscribe);
  };
  open();
  return () => { closed = true; port?.disconnect(); };
}
