import assert from "node:assert/strict";
import { test } from "node:test";
import { pullRequestOf } from "../src/features/launch/pullRequest.ts";

const ref = (url, kind = "comment", repo = "o/r", number = 7) => ({ url, kind, repo, number, title: null });

test("finds a PR from a pasted comment link in the prompt", () => {
  assert.deepEqual(pullRequestOf("please fix https://github.com/O/R/pull/7#discussion_r12", [], ["o/r"]), { repo: "O/R", number: 7 });
});

test("uses context references, including comments on a PR", () => {
  assert.deepEqual(pullRequestOf("", [ref("https://github.com/o/r/pull/7#issuecomment-1")], ["o/r"]), { repo: "o/r", number: 7 });
  assert.equal(pullRequestOf("", [ref("https://github.com/o/r/issues/7#issuecomment-1")], ["o/r"]), null);
});

test("ignores other repositories and stays silent when several PRs match", () => {
  assert.equal(pullRequestOf("https://github.com/x/y/pull/1", [], ["o/r"]), null);
  assert.equal(pullRequestOf("https://github.com/o/r/pull/1 and https://github.com/o/r/pull/2", [], ["o/r", null]), null);
  assert.deepEqual(pullRequestOf("https://github.com/o/r/pull/1 https://github.com/o/r/pull/1/files", [], ["u/r", "o/r"]), { repo: "o/r", number: 1 });
});
