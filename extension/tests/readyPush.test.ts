import { test } from "node:test";
import assert from "node:assert/strict";
import { blockedReason, cardFor, cardsByNote, pushKey, pushTargets } from "../src/shared/readyPush.ts";

const HEAD = "15b9affa7ad8be67d0f613b04ffa26c9c765bac3";
const OTHER = "c42eb98000000000000000000000000000000000";
const card = (over: Record<string, unknown> = {}) => ({ id: "card1", title: "Push the fix", note: null, session_id: "s1", branch: "lp/fix-5d749fb7", head: HEAD, state: "ready", ...over });
const note = (id: string, at: number, over: Record<string, unknown> = {}) => ({ id, author: "Hubert.py Bot", bot_id: "b1", session_id: "s1", by_owner: false, body: "Done in 15b9aff: click Push to this PR", created_at: at, reply_to: null, suggestion: null, unread: false, ...over });
const branch = (p: Record<string, unknown> | null, over: Record<string, unknown> = {}) => ({ branch: "lp/fix-5d749fb7", url: "u", head: HEAD, not_on_github: false, error: null, push_ready: p, ...over });

test("a ready card goes under the newest note of the agent that prepared it, never under the owner's notes", () => {
  const ctx = { notes: [note("n1", 1), note("n2", 3), note("mine", 5, { by_owner: true }), note("other", 4, { session_id: "s2" })], ready_pushes: [card()] } as never;
  assert.deepEqual([...cardsByNote(ctx).keys()], ["n2"]);
  // No session, or no note of it: nothing in the timeline (the sidebar box still offers it).
  assert.equal(cardsByNote({ notes: [note("n1", 1)], ready_pushes: [card({ session_id: null })] } as never).size, 0);
  assert.equal(cardsByNote({ notes: [note("n1", 1, { session_id: "s9" })], ready_pushes: [card()] } as never).size, 0);
});

test("push targets come once per branch and commit, the card joining the agent branch it names", () => {
  const p = { branch: "lp/fix-5d749fb7", ahead: 2, head: HEAD, session_id: "s1", title: "Fix" };
  const both = pushTargets({ branches: [branch(p)], ready_pushes: [card()] } as never);
  assert.equal(both.length, 1);
  assert.equal(both[0]!.card, "card1");
  assert.equal(both[0]!.ahead, 2);
  // A card alone (its agent not on this page) is still a target; a card whose branch moved or was pushed is not.
  assert.deepEqual(pushTargets({ branches: [], ready_pushes: [card()] } as never).map((t) => [t.branch, t.head, t.card, t.ahead]), [["lp/fix-5d749fb7", HEAD, "card1", null]]);
  for (const state of ["moved", "gone", "pushed"]) assert.deepEqual(pushTargets({ branches: [], ready_pushes: [card({ state })] } as never), []);
  // The agent branch at another commit than the card: two pushes, the branch without the card.
  const two = pushTargets({ branches: [branch({ ...p, head: OTHER })], ready_pushes: [card()] } as never);
  assert.deepEqual(two.map((t) => [t.head, t.card]), [[OTHER, null], [HEAD, "card1"]]);
  assert.equal(cardFor({ ready_pushes: [card()] } as never, "lp/fix-5d749fb7", HEAD.toUpperCase())?.id, "card1");
  assert.equal(pushKey({ branch: "lp/x", head: "ABC" }), "lp/x@abc");
});

test("a card that cannot run from the page says why", () => {
  assert.equal(blockedReason({ state: "ready", branch: "lp/x" }), null);
  for (const state of ["moved", "gone", "pushed", "weird"]) assert.ok(blockedReason({ state, branch: "lp/x" }));
});
