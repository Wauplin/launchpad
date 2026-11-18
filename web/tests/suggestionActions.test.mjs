import assert from "node:assert/strict";
import { test } from "node:test";
import { describeAction, parseReviewers, postButton, ranked } from "../src/lib/suggestionActions.ts";

test("the post button names the text and every action, close last", () => {
  const close = { kind: "close", reason: null, duplicate_of: null };
  const reviews = { kind: "request_reviewers", reviewers: ["ana"], team_reviewers: [] };
  assert.equal(postButton("Comment", [close]), "Comment and close");
  assert.equal(postButton("Comment", [close, reviews, { kind: "ready_for_review" }]), "Comment, mark ready for review, request reviews and close");
  assert.equal(postButton("", [{ kind: "close", reason: "not_planned", duplicate_of: null }]), "Close as not planned");
  assert.equal(postButton("Approve and merge (squash)", []), "Approve and merge (squash)");
  assert.deepEqual(ranked([close, { kind: "reopen" }]).map((a) => a.kind), ["reopen", "close"]);
  assert.equal(describeAction({ kind: "close", reason: "duplicate", duplicate_of: 4 }), "close as duplicate of #4");
  assert.equal(describeAction({ kind: "request_reviewers", reviewers: ["ana"], team_reviewers: ["core"] }), "request reviews from @ana, team core");
});

test("typed reviewers become logins and team slugs", () => {
  assert.deepEqual(parseReviewers("@ana, bo  org/core\nteam/docs"), { reviewers: ["ana", "bo"], team_reviewers: ["core", "docs"] });
  assert.deepEqual(parseReviewers(" , "), { reviewers: [], team_reviewers: [] });
});
