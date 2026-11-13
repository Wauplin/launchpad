// Inboxes and lists (M2) for the GitHub layer: the notifications strip and Done all handled, Done/read/Unsubscribe
// clicked on github.com recorded in Launchpad's Inbox (`inbox.observed`, never a GitHub write), the remembered Review
// with agent choice, Add to Launchpad (`folder.clone`) and a Local PR's Publish (`local-pr.publish`, run only by the
// confirm button of the extension's own Publish dialog). Every call goes to Home through `homeJson`.

import type { ExtNotifications } from "@/gen/ExtNotifications";
import type { ExtReviewChoice } from "@/gen/ExtReviewChoice";
import type { ObservedResult } from "@/gen/ObservedResult";
import type { Project } from "@/gen/Project";
import { RequestError, type DoneResult, type LocalPrDetail, type ObservedAction, type PushResult } from "../shared/messages";
import { current, homeJson } from "./home";

const id = (v: unknown, what: string) => {
  if (typeof v !== "string" || !/^[A-Za-z0-9_-]{1,80}$/.test(v)) throw new RequestError(`Not a ${what} ID`);
  return v;
};
const workspace = () => {
  const ws = current().workspace;
  if (!ws) throw new RequestError("Choose a workspace in the Launchpad panel first");
  return ws;
};
const REPO = /^[A-Za-z0-9_.-]{1,100}\/[A-Za-z0-9_.-]{1,100}$/;
/** A thread ID or `owner/repo#number`. */
const ITEM = /^([A-Za-z0-9]{1,40}|[A-Za-z0-9_.-]{1,100}\/[A-Za-z0-9_.-]{1,100}#[1-9]\d{0,9})$/;

export function notifications(): Promise<ExtNotifications> {
  return homeJson<ExtNotifications>(`/api/ext/notifications?workspace=${encodeURIComponent(workspace())}`);
}

const ACTIONS: readonly ObservedAction[] = ["done", "read", "unsubscribe"];
export function observed(a: { items: string[]; action: ObservedAction }): Promise<ObservedResult[]> {
  if (!ACTIONS.includes(a.action)) throw new RequestError("Not an Inbox action");
  const ids = (Array.isArray(a.items) ? a.items : []).map(String).filter((i) => ITEM.test(i)).slice(0, 100);
  if (!ids.length) throw new RequestError("No thread to record");
  return homeJson<ObservedResult[]>("/api/ext/inbox/observed", { method: "POST", body: { ids, action: a.action } });
}

export function doneHandled(a: { ids: string[] }): Promise<DoneResult[]> {
  const ids = (Array.isArray(a.ids) ? a.ids : []).map(String).filter((i) => /^[A-Za-z0-9]{1,40}$/.test(i)).slice(0, 100);
  if (!ids.length) throw new RequestError("No thread to mark Done");
  return homeJson<DoneResult[]>("/api/ext/inbox/done-handled", { method: "POST", body: { ids, workspace: workspace() } });
}

export function reviewChoice(a: { repo: string; bot?: string | null; preset?: string | null }): Promise<ExtReviewChoice> {
  if (typeof a.repo !== "string" || !REPO.test(a.repo)) throw new RequestError("Not a repository");
  const body = { repo: a.repo, workspace: workspace(), bot: a.bot ? id(a.bot, "bot") : null, preset: a.preset ? id(a.preset, "preset") : null };
  return homeJson<ExtReviewChoice>("/api/ext/review-choice", { method: "PUT", body });
}

export function clone(a: { repo: string; workspace_id: string }): Promise<Project> {
  if (typeof a.repo !== "string" || !REPO.test(a.repo)) throw new RequestError("Not a repository");
  const ws = id(a.workspace_id, "workspace");
  if (!current().workspaces.some((w) => w.id === ws)) throw new RequestError("Not a workspace of this Home");
  return homeJson<Project>("/api/projects/clone", { method: "POST", body: { repo: a.repo, workspace_id: ws } });
}

export function localPr(a: { id: string }): Promise<LocalPrDetail> {
  return homeJson<LocalPrDetail>(`/api/predrafts/${id(a.id, "Local PR")}`);
}

/** Publish of the extension's Publish dialog, after its confirmation: the reviewed commit only (Home refuses a
 * branch that moved since). */
export function publish(a: { id: string; remote_branch: string; draft: boolean; head: string; fork: boolean }) {
  const remote = String(a.remote_branch ?? "").trim();
  if (!remote || remote.length > 200 || /\s|\.\.|^[-/]|[~^:?*[\\]/.test(remote)) throw new RequestError("Not a branch name");
  if (typeof a.head !== "string" || !/^[0-9a-f]{40,64}$/.test(a.head)) throw new RequestError("No reviewed commit");
  return homeJson(`/api/predrafts/${id(a.id, "Local PR")}/publish`, { method: "POST", body: { remote_branch: remote, draft: !!a.draft, head: a.head, fork: !!a.fork }, idempotency_key: crypto.randomUUID() });
}

/** Push to this PR of the extension's push dialog, after its confirmation (`branch.push-to-pr`): the branch commit the
 * dialog showed only; Home reads the pull request again and refuses anything but a fast-forward. */
export function pushToPr(a: { repo: string; number: number; branch: string; head: string }): Promise<PushResult> {
  if (typeof a.repo !== "string" || !REPO.test(a.repo)) throw new RequestError("Not a repository");
  const n = Number(a.number);
  if (!Number.isInteger(n) || n < 1) throw new RequestError("Not a pull request");
  const branch = String(a.branch ?? "");
  if (!/^lp\/[A-Za-z0-9._/-]{1,200}$/.test(branch) || branch.includes("..")) throw new RequestError("Not a Launchpad branch");
  if (typeof a.head !== "string" || !/^[0-9a-f]{40}$/.test(a.head)) throw new RequestError("No branch commit");
  const [owner, name] = a.repo.split("/");
  return homeJson<PushResult>(`/api/items/${owner}/${name}/${n}/push-branch`, { method: "POST", body: { branch, head: a.head } });
}
