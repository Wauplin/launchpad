// Conversations of several bots (`launchpad_core::bot_threads`): the owner follows one conversation while each bot
// keeps its own thread (its own session). The bot page shows them merged: one timeline of every bot's thread, the
// owner's messages with the bots they went to, each bot's answers in its color, bot-to-bot messages once. The owner's
// message goes to the conversation's main bot, or to the bots named by the @-mentions it starts with. Pure, so it is
// tested (tests/conversations.test.mjs).

import type { Session } from "../gen/Session";
import { compareNames } from "./channelRows.ts";
import { isQuotaPaused, isRunning, isSleeping } from "./sessionStatus.ts";

/** A bot as the conversation helpers need it. */
export type ConversationBot = { id: string; slug: string; name: string };

/** The main bot of a conversation (`<bot>:<thread>`). */
export const mainBotOf = (conversation: string) => conversation.split(":")[0] ?? conversation;

/** The leading run of @-mentions: "@a @b, text" → ["a", "b"]; mentions after the text starts do not count. */
const LEADING = /^\s*((?:@[\w.-]+[\s,]*)+)/;

/** Where the owner's message goes: the main bot when it starts with no @-mention, else each bot named by the
 * mentions it starts with (in order, each once; a mention after the text starts does not route). An unknown name
 * is refused before anything is sent. The text goes as typed. */
export function routeMentions(text: string, bots: readonly ConversationBot[], mainId: string): { targets: string[] } | { error: string } {
  const run = LEADING.exec(text);
  if (!run) return { targets: [mainId] };
  const names = run[1].match(/@[\w.-]+/g)!.map((m) => m.slice(1));
  const targets: string[] = [];
  for (const n of names) {
    const lower = n.toLowerCase();
    const bot = bots.find((b) => b.slug.toLowerCase() === lower) ?? bots.find((b) => b.name.toLowerCase().replace(/\s+/g, "-") === lower);
    if (!bot) return { error: `@${n}: no bot of this workspace has that name` };
    if (!targets.includes(bot.id)) targets.push(bot.id);
  }
  if (!text.slice(run[0].length).trim()) return { error: "Write your message after the @-mentions" };
  return { targets };
}

/** The @-mention being typed in the leading run (the text before the caret), or null: "@sc" → "sc", "@a @" → "". */
export function mentionQuery(before: string): string | null {
  const m = /^\s*(?:@[\w.-]+[\s,]+)*@([\w.-]*)$/.exec(before);
  return m ? m[1] : null;
}

/** The bots the @ suggestions offer for a query: participants first, then the workspace's other bots, by name. */
export function mentionChoices(query: string, bots: readonly ConversationBot[], participants: readonly string[], max = 6): ConversationBot[] {
  const q = query.toLowerCase();
  return bots
    .filter((b) => b.slug.toLowerCase().startsWith(q) || b.name.toLowerCase().startsWith(q))
    .sort((a, b) => Number(participants.includes(b.id)) - Number(participants.includes(a.id)) || compareNames(a.name, b.name))
    .slice(0, max);
}

/** The text with the mention being typed (`before` ends with it) replaced by `@slug `. */
export function completeMention(text: string, before: string, slug: string): string {
  const at = before.lastIndexOf("@");
  return `${text.slice(0, at)}@${slug} ${text.slice(before.length).replace(/^\s+/, "")}`;
}

/** One bot's state in a conversation's status line. */
function botState(s: Session | null): { word: string; rank: number } {
  if (!s) return { word: "waiting", rank: 5 };
  if (s.open_question || isQuotaPaused(s)) return { word: "waits for you", rank: 0 };
  if (s.status === "error" || s.status === "interrupted") return { word: s.status === "error" ? "failed" : "interrupted", rank: 1 };
  if (isRunning(s)) return { word: "working", rank: 2 };
  if (isSleeping(s)) return { word: "sleeping", rank: 3 };
  return { word: "waiting", rank: 4 };
}

/** The status line of a conversation, most urgent first: "B working · C waits for you · A waiting". */
export function conversationStatus(parts: readonly { name: string; session: Session | null }[]): string {
  return parts
    .map((p) => ({ name: p.name, ...botState(p.session) }))
    .sort((a, b) => a.rank - b.rank || a.name.localeCompare(b.name))
    .map((p) => `${p.name} ${p.word}`)
    .join(" · ");
}

/** What the merge needs of a chat item (`chatRows.ts`'s ChatItem). */
type Item = { key: string; v: string; t: string; at?: number | null; text?: string | null; from?: { kind: string; id: string; reply?: boolean } | null; bot?: { id: string; name: string }; to?: string[] };

/** Items that start a part of the timeline: a message, a routine run, a new session. */
const STARTS = new Set(["divider", "user", "from", "wake", "routine", "quiet", "launch"]);
/** The owner's same message to several bots shows once when they arrive this close together. */
const SAME_MESSAGE_MS = 10_000;

/** The merged timeline of a conversation: each bot's chat items (`chatRows` of its thread), cut into parts at each
 * message and ordered by time (a part keeps its order: a message, its answer, its steps, its questions). Each item
 * names its bot; the owner's messages name the bots they went to (`to`, the same message to several bots once), a
 * message from another bot of the conversation names its receiver, and an answer to such a message names whom it
 * answers. An answer that came back to a bot (`from.reply`) is left out: it shows as its author's answer. Only the
 * main bot's sessions get their dividers. Waiting rows (pending, queued) go last. */
export function mergeConversation<T extends Item>(parts: readonly { bot: { id: string; name: string }; items: readonly T[] }[], mainId: string): T[] {
  const ids = new Map(parts.map((p) => [p.bot.id, p.bot.name]));
  type Part = { at: number; order: number; items: T[] };
  const segments: Part[] = [];
  const tail: T[] = [];
  let order = 0;
  for (const p of parts) {
    let cur = null as Part | null;
    let asker: string | null = null;
    for (const raw of p.items) {
      if (raw.t === "divider" && p.bot.id !== mainId) continue;
      const item = { ...raw, bot: p.bot } as T;
      if (raw.t === "pending" || raw.t === "queued") { tail.push({ ...item, to: [p.bot.name], v: `${raw.v}|${p.bot.name}` } as T); continue; }
      if (STARTS.has(raw.t) || !cur) {
        const at: number = raw.at ?? cur?.at ?? 0;
        cur = { at, order: order++, items: [] };
        segments.push(cur);
        asker = null;
      }
      // An answer that came back starts the turn it caused, but shows only as its author's answer.
      if (raw.t === "from" && raw.from?.reply) continue;
      if (raw.t === "user") { cur.items.push({ ...item, to: [p.bot.name] } as T); continue; }
      if (raw.t === "from" && raw.from?.kind === "bot" && ids.has(raw.from.id)) {
        asker = ids.get(raw.from.id)!;
        cur.items.push({ ...item, to: [p.bot.name], v: `${raw.v}|${p.bot.name}` } as T);
        continue;
      }
      if (raw.t === "answer" && asker) { cur.items.push({ ...item, to: [asker], v: `${raw.v}|${asker}` } as T); continue; }
      cur.items.push(item);
    }
  }
  segments.sort((a, b) => a.at - b.at || a.order - b.order);
  const out: T[] = [];
  // The owner's messages seen so far, to fold the same message to several bots into one.
  const owner: T[] = [];
  for (const s of segments) for (const item of s.items) {
    if (item.t === "user") {
      const same = owner.find((o) => o.text === item.text && Math.abs((o.at ?? 0) - (item.at ?? 0)) <= SAME_MESSAGE_MS);
      if (same) {
        const to = [...(same.to ?? []), ...(item.to ?? [])];
        Object.assign(same, { to, v: `${same.v.split("|")[0]}|${to.join(",")}` });
        continue;
      }
      const first = { ...item, v: `${item.v}|${(item.to ?? []).join(",")}` } as T;
      owner.push(first);
      out.push(first);
      continue;
    }
    out.push(item);
  }
  // Your same message waiting for several bots shows once too.
  const waiting: T[] = [];
  for (const item of tail) {
    const same = item.t === "pending" && waiting.find((w) => w.t === "pending" && w.text === item.text);
    if (same) { Object.assign(same, { to: [...(same.to ?? []), ...(item.to ?? [])], v: `${same.v},${item.bot?.name}` }); continue; }
    waiting.push(item);
  }
  return [...out, ...waiting];
}
