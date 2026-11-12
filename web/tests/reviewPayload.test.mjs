import assert from "node:assert/strict";
import { test } from "node:test";
import { reviewPayload } from "../src/features/github/reviewPayload.ts";

test("one review includes current line comments with GitHub sides, and approval-only merge", () => {
  const draft = { summary: "Summary", source: "agent-draft", merge: true, runCi: false, comments: [
    { path: "a.ts", line: 5, side: "old", text: "Removed", head: "current" },
    { path: "b.ts", line: 8, side: "new", text: "Added", head: "current" },
    { path: "a.ts", line: 9, text: "Outdated", head: "old" },
  ] };
  const payload = reviewPayload(draft, "current", "APPROVE");
  assert.equal(payload.body, "Summary");
  assert.equal(payload.source, "agent-draft");
  assert.equal(payload.commit_id, "current");
  assert.deepEqual(payload.comments, [
    { path: "a.ts", line: 5, side: "LEFT", body: "Removed" },
    { path: "b.ts", line: 8, side: "RIGHT", body: "Added" },
  ]);
  assert.equal(payload.merge, true);
  assert.equal(reviewPayload(draft, "current", "REQUEST_CHANGES").merge, false);
});

test("a line comment over a range keeps its first line", () => {
  const draft = { summary: "", source: null, merge: false, runCi: false, comments: [{ path: "a.ts", line: 9, startLine: 7, side: "new", text: "```suggestion\nx\n```" }] };
  assert.deepEqual(reviewPayload(draft, "h", "COMMENT").comments, [{ path: "a.ts", line: 9, side: "RIGHT", body: "```suggestion\nx\n```", start_line: 7 }]);
});
