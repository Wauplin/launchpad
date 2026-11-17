import { test } from "node:test";
import assert from "node:assert/strict";
import { submitSuggestion } from "../src/content/github/suggestionAction.ts";

test("comment close and review post reuse suggestion.post payload without overriding merge head", async () => {
  for (const [kind, close] of [["comment", true], ["review", false]] as const) {
    let sent: unknown;
    await submitSuggestion({ id: "draft", kind, status: "draft" }, close, async (args) => { sent = args; });
    assert.deepEqual(sent, { id: "draft", close });
  }
});
test("inserted, uncertain and already attempted suggestions never submit", async () => {
  for (const status of ["inserted", "uncertain", "posting", "posted"]) {
    await assert.rejects(submitSuggestion({ id: "x", kind: "comment", status }, true, async () => assert.fail("sent")));
  }
});
test("reviews cannot close unmerged and request errors propagate", async () => {
  await assert.rejects(submitSuggestion({ id: "x", kind: "review", status: "draft" }, true, async () => assert.fail("sent")));
  await assert.rejects(submitSuggestion({ id: "x", kind: "review", status: "draft" }, false, async () => { throw new Error("PR head changed"); }), /PR head changed/);
});

test("duplicate requests are blocked while publication is pending", async () => {
  let finish!: () => void;
  const s = { id: "pending", kind: "comment", status: "draft" };
  const first = submitSuggestion(s, false, () => new Promise<void>((resolve) => { finish = resolve; }));
  await assert.rejects(submitSuggestion(s, false, async () => assert.fail("duplicate")), /already in progress/);
  finish();
  await first;
});
