// Replies grouped under the message they answer, one level deep, folded by default: the chat stays in time order for
// every other message, and a request shows "2 replies · 10:07" under it. Only explicit links count (a channel
// message's `reply_to`, a bot's answer naming its request): never a guess from the order of messages. A reply whose
// parent is not loaded (an older page, or no link) stays an ordinary message at its time. A reply to a reply joins the
// group of the first message of its chain, so nesting never goes deeper. Pure, so it is tested
// (tests/replyGroups.test.mjs).

/** How to read the links of a list's items. */
export type ReplyLinks<T> = {
  /** The ID other items may answer, else null. */
  id: (item: T) => string | null;
  /** The ID of the message this item answers, when it is a reply to group, else null. */
  parent: (item: T) => string | null;
};

export type ReplyGroups<T> = {
  /** The items shown at their time, in the list's order. */
  top: T[];
  /** The replies of each top item (by its ID), in the list's order. */
  replies: Map<string, T[]>;
  /** The top item's ID each grouped reply shows under. */
  rootOf: Map<T, string>;
};

/** Splits a list (oldest first) into the items shown at their time and the replies grouped under them. */
export function groupReplies<T>(items: readonly T[], links: ReplyLinks<T>): ReplyGroups<T> {
  const byId = new Map<string, T>();
  for (const it of items) {
    const id = links.id(it);
    if (id != null && !byId.has(id)) byId.set(id, it);
  }
  // The first message of a reply's chain that is not itself a grouped reply; null when the parent is not loaded or
  // the chain loops.
  const root = (item: T): string | null => {
    let p = links.parent(item);
    if (p == null || !byId.has(p)) return null;
    const seen = new Set<T>([item]);
    for (;;) {
      const node = byId.get(p)!;
      if (seen.has(node)) return null;
      seen.add(node);
      const up = links.parent(node);
      if (up == null || !byId.has(up)) return p;
      p = up;
    }
  };
  const top: T[] = [];
  const replies = new Map<string, T[]>();
  const rootOf = new Map<T, string>();
  for (const it of items) {
    const r = root(it);
    if (r == null) { top.push(it); continue; }
    rootOf.set(it, r);
    const list = replies.get(r);
    if (list) list.push(it); else replies.set(r, [it]);
  }
  return { top, replies, rootOf };
}

/** One row of the shown list: an item (`nested` under its request), or the fold of a request's replies. */
export type ReplyRow<T> =
  | { kind: "item"; item: T; nested: boolean }
  | { kind: "fold"; parent: string; replies: T[]; open: boolean };

/** The rows to draw: each top item, then the fold of its replies, then (when open) the replies themselves. */
export function replyRows<T>(g: ReplyGroups<T>, links: ReplyLinks<T>, isOpen: (parent: string) => boolean): ReplyRow<T>[] {
  const out: ReplyRow<T>[] = [];
  for (const it of g.top) {
    out.push({ kind: "item", item: it, nested: false });
    const id = links.id(it);
    const list = id == null ? undefined : g.replies.get(id);
    if (!id || !list?.length) continue;
    const open = isOpen(id);
    out.push({ kind: "fold", parent: id, replies: list, open });
    if (open) for (const r of list) out.push({ kind: "item", item: r, nested: true });
  }
  return out;
}

/** "1 reply", "3 replies". */
export const repliesLabel = (n: number) => `${n} repl${n === 1 ? "y" : "ies"}`;
