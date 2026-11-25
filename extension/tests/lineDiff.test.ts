import { test } from "node:test";
import assert from "node:assert/strict";
import { lineDiff, wordDiff } from "../src/content/github/lineDiff.ts";

const show = (rows: ReturnType<typeof lineDiff>) => rows.map((r) => ({ del: "-", add: "+", same: " " })[r.kind] + r.parts.map((p) => (p.changed ? `[${p.text}]` : p.text)).join(""));

test("a one-word edit in a long line marks only that word", () => {
  assert.deepEqual(show(lineDiff(["Run the job on a GPU flavor."], ["Run the job on a CPU flavor."])), ["-Run the job on a [GPU] flavor.", "+Run the job on a [CPU] flavor."]);
});
test("kept lines stay plain, unpaired lines are whole, unrelated lines get no word marks", () => {
  assert.deepEqual(show(lineDiff(["a", "b"], ["a", "b", "c"])), [" a", " b", "+c"]);
  assert.deepEqual(show(lineDiff(["one"], [])), ["-one"]);
  assert.deepEqual(wordDiff("alpha beta", "gamma delta"), { del: [{ text: "alpha beta", changed: false }], add: [{ text: "gamma delta", changed: false }] });
});
