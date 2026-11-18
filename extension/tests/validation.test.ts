import { test } from "node:test";
import assert from "node:assert/strict";
import { checkedActions, id, workspace } from "../src/background/validation.ts";

test("request IDs retain the exact allowed alphabet and length boundaries", () => {
  for (const value of [null, undefined, 3, {}, "", "a".repeat(81), "a/b", "a.b", " a", "a\n", "é"]) {
    assert.throws(() => id(value, "bot"), { message: "Not a bot ID" });
  }
  for (const value of ["a", "A_0-z", "a".repeat(80)]) assert.equal(id(value, "bot"), value);
});

test("workspace selection fails before a route can use a missing workspace", () => {
  for (const value of [null, ""]) assert.throws(() => workspace(value), { message: "Choose a workspace in the Launchpad panel first" });
  assert.equal(workspace("work"), "work");
});

test("the actions a page sends are rebuilt from known kinds and values only", () => {
  assert.deepEqual(checkedActions([{ kind: "close", reason: "not_planned", extra: 1 }, { kind: "request_reviewers", reviewers: ["ana"], team_reviewers: ["core"] }, { kind: "reopen", x: "y" }]), [
    { kind: "close", reason: "not_planned", duplicate_of: null },
    { kind: "request_reviewers", reviewers: ["ana"], team_reviewers: ["core"] },
    { kind: "reopen" },
  ]);
  for (const bad of [null, {}, [{ kind: "merge" }], [{ kind: "close", reason: "spam" }], [{ kind: "close", duplicate_of: -1 }], [{ kind: "request_reviewers", reviewers: ["a b"] }], [{ kind: "request_reviewers", reviewers: "ana" }]]) {
    assert.throws(() => checkedActions(bad));
  }
});
