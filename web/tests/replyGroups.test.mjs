import assert from "node:assert/strict";
import { test } from "node:test";
import { groupReplies, repliesLabel, replyRows } from "../src/lib/replyGroups.ts";

// Channel-like messages: `id`, the message a bot reply answers (`to`), and whether it is a bot's.
const links = { id: (m) => String(m.id), parent: (m) => (m.bot && m.to != null ? String(m.to) : null) };
const m = (id, to = null, bot = true) => ({ id, to, bot });
const show = (rows) => rows.map((r) => (r.kind === "fold" ? `[${r.parent}: ${r.replies.length}${r.open ? " open" : ""}]` : `${r.nested ? "  " : ""}${r.item.id}`));

test("replies fold under their request; other messages keep their order", () => {
  const list = [m(1, null, false), m(2), m(3), m(4, null, false), m(5, 3), m(6, 2), m(7)];
  const g = groupReplies(list, links);
  assert.deepEqual(show(replyRows(g, links, () => false)), ["1", "2", "[2: 1]", "3", "[3: 1]", "4", "7"]);
  assert.deepEqual(show(replyRows(g, links, (p) => p === "3")), ["1", "2", "[2: 1]", "3", "[3: 1 open]", "  5", "4", "7"]);
  assert.equal(g.rootOf.get(list[4]), "3");
});

test("a reply to a reply joins the first message's group: one level only", () => {
  const list = [m(1), m(2, 1), m(3, 2), m(4, 3)];
  assert.deepEqual(show(replyRows(groupReplies(list, links), links, () => true)), ["1", "[1: 3 open]", "  2", "  3", "  4"]);
});

test("an unloaded parent, the owner's reply or a loop leaves the message at its time", () => {
  const list = [m(5, 2), m(6), m(7, 6, false), m(8, 9), m(9, 8)];
  assert.deepEqual(show(replyRows(groupReplies(list, links), links, () => false)), ["5", "6", "7", "8", "9"]);
  assert.equal(repliesLabel(1), "1 reply");
  assert.equal(repliesLabel(2), "2 replies");
});
