import assert from "node:assert/strict";
import { test } from "node:test";
import { canMergeWorktree } from "../src/features/agents/worktreeMerge.ts";

const saved = (worktree = {}, info = {}) => ({
  worktree: { state: "removed", base_branch: "main", predraft_id: "pr", ...worktree },
  on_home: true, ahead: 1, head: "abc123", away: "Its Burst runner has stopped", ...info,
});
const localPr = { id: "pr", published_url: null };

test("a saved Burst branch with an unpublished Local PR offers Merge", () => {
  assert.equal(canMergeWorktree(saved(), [localPr]), true);
  assert.equal(canMergeWorktree(saved({ state: "active" }), [localPr]), true);
  assert.equal(canMergeWorktree(saved({ predraft_id: null })), true);
});

test("published pull requests and unknown Local PRs do not offer a worktree merge", () => {
  assert.equal(canMergeWorktree(saved(), [{ ...localPr, published_url: "https://github.com/owner/repo/pull/1" }]), false);
  assert.equal(canMergeWorktree(saved(), []), false);
  assert.equal(canMergeWorktree(saved()), false);
  assert.equal(canMergeWorktree(saved({ pull_request: { number: 1 } }), [localPr]), false);
});

test("Merge still needs an available branch, new commits, a head and no pending merge", () => {
  for (const info of [{ on_home: false }, { ahead: 0 }, { head: null }]) {
    assert.equal(canMergeWorktree(saved({}, info), [localPr]), false);
  }
  for (const worktree of [{ state: "discarded" }, { state: "preparing" }, { base_branch: null }, { merge: { phase: "waiting" } }]) {
    assert.equal(canMergeWorktree(saved(worktree), [localPr]), false);
  }
});
