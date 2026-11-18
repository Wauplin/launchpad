import { test } from "node:test";
import assert from "node:assert/strict";
import { id, workspace } from "../src/background/validation.ts";

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
