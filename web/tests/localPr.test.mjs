import assert from "node:assert/strict";
import { test } from "node:test";
import { githubPr, localPrOf } from "../src/features/predrafts/localPrState.ts";

test("a published Local PR names its GitHub pull request", () => {
  assert.deepEqual(githubPr({ published_url: "https://github.com/huggingface/huggingface_hub/pull/5342", published_number: 5342 }), { repo: "huggingface/huggingface_hub", number: 5342 });
  assert.deepEqual(githubPr({ published_url: "https://github.com/o/r/pull/7", published_number: null }), { repo: "o/r", number: 7 });
  assert.equal(githubPr({ published_url: null, published_number: null }), null);
});

test("a worktree's Local PR: recorded first, else the same folder and branch, never a discarded one", () => {
  const prs = [
    { id: "old", project_id: "f", branch: "lp/x", archived: true },
    { id: "b", project_id: "f", branch: "lp/x", archived: false },
    { id: "other", project_id: "g", branch: "lp/x", archived: false },
  ];
  assert.equal(localPrOf({ predraft_id: "b", project_id: "f", branch: "lp/y" }, prs)?.id, "b");
  assert.equal(localPrOf({ predraft_id: null, project_id: "f", branch: "lp/x" }, prs)?.id, "b");
  assert.equal(localPrOf({ predraft_id: "old", project_id: "f", branch: "lp/x" }, prs)?.id, "b");
  assert.equal(localPrOf({ predraft_id: null, project_id: "h", branch: "lp/x" }, prs), undefined);
  assert.equal(localPrOf({ predraft_id: null, project_id: "f", branch: "lp/x" }, undefined), undefined);
});
