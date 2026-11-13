// What the features of a pull request or issue page share: the page's context, the workspace's bots, the agents
// asked from this page before Home's context names them, and the actions (ask a bot, stop an agent, open Launchpad,
// record a suggestion's fate). References only: the item and comment IDs come from the URL and the page's anchor IDs,
// never from comment text.

import type { ExtAgent } from "@/gen/ExtAgent";
import type { ExtContext } from "@/gen/ExtContext";
import type { ExtSuggestion } from "@/gen/ExtSuggestion";
import type { GitHubRef } from "@/gen/GitHubRef";
import { request, type BotBrief, type LedgerAction } from "../../shared/messages";
import type { Page } from "./page";

export type ItemPage = Page & { repo: string; number: number };

export const isItem = (p: Page): p is ItemPage => !!p.repo && !!p.number && (p.kind === "pr_conversation" || p.kind === "issue");

/** The item's URL without its fragment or tab (`/pull/12`, `/issues/7`). */
export function itemUrl(p: ItemPage) {
  return `https://github.com/${p.repo}/${p.kind === "issue" ? "issues" : "pull"}/${p.number}`;
}

export function open(path: string, ws?: string | null) {
  const full = ws ? `${path}${path.includes("?") ? "&" : "?"}ws=${encodeURIComponent(ws)}` : path;
  void request("open", { path: full }).catch((e) => console.warn("Launchpad: open", e));
}

export const agentPath = (a: { session_id: string }) => `/agents?session=${encodeURIComponent(a.session_id)}`;

// Bots of the workspace, kept a minute per page.
let botsCache: { at: number; list: Promise<{ bots: BotBrief[]; ask: string | null }> } | null = null;
export function workspaceBots() {
  if (!botsCache || Date.now() - botsCache.at > 60_000) {
    const list = request("bots");
    list.catch(() => { botsCache = null; });
    botsCache = { at: Date.now(), list };
  }
  return botsCache.list;
}

/** Agents asked from this tab whose context did not name them yet: comment anchor → agent, shown at once. */
export const asked = new Map<string, ExtAgent>();

/** The agent of a comment's live card: Home's (the newest that was asked about it) or the one just asked here. */
export function agentOf(ctx: ExtContext | null, comment: string): ExtAgent | null {
  const fromHome = ctx?.agents.find((a) => a.comment === comment) ?? null;
  const local = asked.get(comment) ?? null;
  if (fromHome && local && fromHome.session_id === local.session_id) asked.delete(comment);
  return fromHome && (!local || fromHome.since >= local.since) ? fromHome : local ?? fromHome;
}

/** A suggestion that answers this comment (a reply to it, or written by the agent asked about it). */
export function replyFor(ctx: ExtContext | null, comment: string, agent: ExtAgent | null): ExtSuggestion | null {
  const list = (ctx?.suggestions ?? []).filter((s) => s.kind === "comment");
  const anchor = (s: ExtSuggestion) => s.reply_to ? (s.reply_to.kind === "review_comment" ? `discussion_r${s.reply_to.id}` : `issuecomment-${s.reply_to.id}`) : null;
  return list.find((s) => anchor(s) === comment) ?? (agent ? list.find((s) => s.session_id === agent.session_id && !s.reply_to) ?? null : null);
}

export const running = (a: ExtAgent) => a.status === "working" || a.status === "starting" || a.status === "background";

/** "3 min", "2 h", "1 d". */
export function elapsed(since: number) {
  const m = Math.max(0, Math.round((Date.now() - since) / 60_000));
  if (m < 1) return "now";
  if (m < 60) return `${m} min`;
  const hours = Math.round(m / 60);
  return hours < 48 ? `${hours} h` : `${Math.round(hours / 24)} d`;
}

export function statusText(a: ExtAgent) {
  if (a.open_question) return "asks you";
  if (running(a)) return "working";
  switch (a.status) {
    case "waiting": return a.needs_you ? "done · new" : "done";
    case "error": return "failed";
    case "interrupted": return "interrupted";
    case "quota_paused": return "paused for quota";
    case "stopped": return "stopped";
    default: return a.status.replace(/_/g, " ");
  }
}

export function dotClass(a: ExtAgent) {
  if (a.open_question || a.status === "quota_paused") return "lp-dot lp-need";
  if (running(a)) return "lp-dot lp-run";
  if (a.status === "error" || a.status === "interrupted") return "lp-dot lp-bad";
  if (a.status === "waiting") return "lp-dot lp-ok";
  return "lp-dot";
}

/** The reference of the item, or of one of its comments (`issuecomment-…`, `discussion_r…`). */
export function refOf(p: ItemPage, ctx: ExtContext | null, comment?: string | null): GitHubRef {
  const url = itemUrl(p);
  const title = ctx?.page.title ?? null;
  if (comment) return { url: `${url}#${comment}`, kind: comment.startsWith("discussion_r") ? "review_comment" : "comment", repo: p.repo, number: p.number, title };
  return { url, kind: p.kind === "issue" ? "issue" : "pr", repo: p.repo, number: p.number, title };
}

/** The message of an Ask: the owner's note, then how to answer so the reply lands back here. */
export function askText(p: ItemPage, note: string, comment: string | null, kind: "ask" | "review" | "reply") {
  const target = `--repo ${p.repo} --number ${p.number}`;
  if (kind === "review") return `${note || "Review this pull request."}\n\nPut your findings in a suggested review (\`launchpad suggest review ${target} --head <commit you reviewed>\`, line comments with --comments-file) for me to post.`;
  if (!comment && kind === "ask") return `${note || "Look at this item again: what changed, and what should I do?"}\n\nWhen something should be posted, draft it as a suggestion (\`launchpad suggest comment|review ${target} …\`) for me to post.`;
  if (!comment) return `${note || `Draft a reply to this ${p.kind === "issue" ? "issue" : "pull request"}.`}\n\nIf a reply helps, draft it as a suggested comment (\`launchpad suggest comment ${target} --body-file <file>\`); I insert it into GitHub's comment box and post it myself.`;
  return `${note || "What should I answer to this comment?"}\n\nIf a reply helps, draft it as a suggested comment that answers this comment (\`launchpad suggest comment ${target} --reply-to ${comment} --body-file <file>\`); I insert it into GitHub's reply box and post it myself.`;
}

export async function ask(p: ItemPage, ctx: ExtContext | null, bot: BotBrief, note: string, comment: string | null, kind: "ask" | "review" | "reply") {
  const session = await request("bot.message", { bot: bot.id, text: askText(p, note.trim(), comment, kind), github_refs: [refOf(p, ctx, comment)], key: crypto.randomUUID() });
  const agent: ExtAgent = { session_id: session.id, title: session.title, status: session.status, needs_you: false, open_question: false, bot_id: bot.id, bot: bot.name, thread: session.launch_spec.bot?.thread || "main", number: p.number, since: Date.now(), comment, last_message: null };
  if (comment) asked.set(comment, agent);
  return agent;
}

export function stop(a: ExtAgent) {
  return request("session.stop", { id: a.session_id });
}

export function ledger(action: LedgerAction, id: string, text?: string) {
  return request("suggestion.ledger", { action, id, part: "body", ...(text !== undefined ? { text } : {}) });
}

/** "Hubert.js", "an agent". */
export const who = (a: { bot: string | null } | null) => a?.bot ?? "An agent";
