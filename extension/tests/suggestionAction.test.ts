import { test } from "node:test";
import assert from "node:assert/strict";
import { submitSuggestion } from "../src/content/github/suggestionAction.ts";

const close = { kind: "close", reason: null, duplicate_of: null } as const;
const ready = { kind: "ready_for_review" } as const;

test("comments, reviews and action-only suggestions send the ticked actions through suggestion.post", async () => {
  for (const [kind, actions] of [["comment", [close]], ["review", [ready]], ["action", [close]], ["comment", []]] as const) {
    let sent: unknown;
    await submitSuggestion({ id: "draft", kind, status: "draft" }, [...actions], async (args) => { sent = args; });
    assert.deepEqual(sent, { id: "draft", actions });
  }
});
test("inserted, uncertain and already attempted suggestions never submit", async () => {
  for (const status of ["inserted", "uncertain", "posting", "posted"]) {
    await assert.rejects(submitSuggestion({ id: "x", kind: "comment", status }, [close], async () => assert.fail("sent")));
  }
});
test("reviews cannot close unmerged, an action-only suggestion needs an action, and request errors propagate", async () => {
  await assert.rejects(submitSuggestion({ id: "x", kind: "review", status: "draft" }, [close], async () => assert.fail("sent")));
  await assert.rejects(submitSuggestion({ id: "x", kind: "action", status: "draft" }, [], async () => assert.fail("sent")), /unticked/);
  await assert.rejects(submitSuggestion({ id: "x", kind: "review", status: "draft" }, [], async () => { throw new Error("PR head changed"); }), /PR head changed/);
});

test("duplicate requests are blocked while publication is pending", async () => {
  let finish!: () => void;
  const s = { id: "pending", kind: "comment", status: "draft" };
  const first = submitSuggestion(s, [], () => new Promise<void>((resolve) => { finish = resolve; }));
  await assert.rejects(submitSuggestion(s, [], async () => assert.fail("duplicate")), /already in progress/);
  finish();
  await first;
});
