import assert from "node:assert/strict";
import { test } from "node:test";
import { isWaiting, visibleProposals } from "../src/features/proposals/proposalState.ts";

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

test("a compact card says what it does, and a handled one is a receipt", async () => {
  const { proposalPurpose, proposalReceipt } = await import("../src/features/proposals/proposalState.ts");
  const files = { kind: "bot_files", bot_id: "b", files: { "routines/review.toml": 'title = "Review requests"\n', "BOT.md": "# M\n" }, before: { "BOT.md": "x", "routines/review.toml": null }, patches: {} };
  assert.equal(proposalPurpose({ body: files }, { bot: () => "Maintainer" }), "Changes BOT.md and adds the routine Review requests of Maintainer");
  const routine = { kind: "routine", bot_id: "b", title: "Daily", instructions: "", trigger: { kind: "schedule" }, schedule: "0 9 * * *", conditions: [] };
  assert.equal(proposalReceipt({ body: routine, state: "started", title: "Review requests", replaced_by: null }), "Added routine Review requests");
  assert.equal(proposalReceipt({ body: routine, state: "dismissed", title: "Daily", replaced_by: null }), "Dismissed Daily");
  assert.equal(proposalReceipt({ body: routine, state: "proposed", title: "Daily", replaced_by: null }), null);
  assert.equal(proposalPurpose({ body: { kind: "report", outcome: "needs_you", github: "o/r#1", body: "" } }), "Needs you · o/r#1");
});
