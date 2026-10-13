import assert from "node:assert/strict";
import { test } from "node:test";

const storage = () => ({ getItem: () => null, setItem: () => {} });
globalThis.location = { search: "" };
globalThis.window = { addEventListener: () => {} };
globalThis.sessionStorage = storage();
globalThis.localStorage = storage();
const { emptySpec, rememberProjectDefaults, rememberProjectCompletion, useComposer } = await import("../src/stores/composer.ts");
const { api } = await import("../src/lib/api.ts");
api.put = async () => {};
const draft = () => useComposer.getState();
const merge = () => draft().spec.worktree?.merge_when_done;

test("new drafts and project switches use separate defaults; context additions preserve unsent choices", () => {
  rememberProjectDefaults([{ id: "review", merge_when_done: false }, { id: "merge", merge_when_done: true }]);
  useComposer.setState({ spec: emptySpec(), open: false });
  draft().start({ projectId: "merge" });
  assert.equal(merge(), true);
  draft().update((s) => ({ ...s, target: { kind: "project", project_id: "review" } }));
  assert.equal(merge(), false);
  draft().update((s) => ({ ...s, target: { kind: "project", project_id: "merge" } }));
  assert.equal(merge(), true);
  draft().update((s) => ({ ...s, worktree: { ...s.worktree, merge_when_done: false } }));
  draft().start({ projectId: "merge", githubRefs: [] });
  assert.equal(merge(), false);
  rememberProjectCompletion("merge", false);
  draft().discard();
  draft().start({ projectId: "merge" });
  assert.equal(merge(), false);
  draft().start({ projectId: "unknown" });
  assert.equal(merge(), false);
});

test("restored drafts keep their choice, and newly enabled worktrees use the project default", async () => {
  const saved = { ...emptySpec(), target: { kind: "project", project_id: "merge" } };
  api.get = async (path) => path === "/api/projects" ? [{ id: "merge", merge_when_done: true }] : path === "/api/sessions" ? [] : { spec: saved, open: true };
  await draft().restore();
  assert.equal(merge(), false);
  draft().start();
  assert.equal(merge(), false);
  draft().update((s) => ({ ...s, worktree: null }));
  draft().update((s) => ({ ...s, worktree: emptySpec().worktree }));
  assert.equal(merge(), true);
  draft().update((s) => ({ ...s, target: { kind: "predraft", project_id: "merge" }, worktree: { ...s.worktree, merge_when_done: false } }));
  assert.equal(merge(), false);
});
