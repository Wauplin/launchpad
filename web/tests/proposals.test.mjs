import assert from "node:assert/strict";
import { test } from "node:test";
import { isWaiting, visibleProposals } from "../src/features/ask/proposalState.ts";

test("replacement leaves only its successor waiting and preserves the history", () => {
  const cards = [{ id: "new", state: "proposed" }, { id: "old", state: "replaced" }];
  assert.deepEqual(visibleProposals(cards, true, null), [cards[0]]);
  assert.deepEqual(visibleProposals(cards, false, null), cards);
  for (const state of ["started", "starting", "dismissed", "replaced"]) assert.equal(isWaiting({ state }), false);
  for (const state of ["proposed", "failed", "uncertain"]) assert.equal(isWaiting({ state }), true);
});

test("following a replacement reveals successors outside the recent history", () => {
  const cards = Array.from({ length: 60 }, (_, i) => ({ id: String(i), state: "replaced" }));
  assert.equal(visibleProposals(cards, false, null).length, 50);
  assert.deepEqual(visibleProposals(cards, false, "59").at(-1), cards[59]);
});
