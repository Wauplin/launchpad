import assert from "node:assert/strict";
import { test } from "node:test";

globalThis.setInterval = () => 0;
const storage = () => ({ getItem: () => null, setItem: () => {} });
globalThis.location = { search: "" };
globalThis.window = { addEventListener: () => {} };
globalThis.sessionStorage = storage();
globalThis.localStorage = storage();
const { emptySpec, rememberProjectDefaults, rememberProjectCompletion, useComposer } = await import("../src/stores/composer.ts");
const { api } = await import("../src/lib/api.ts");
api.put = async () => ({ revision: 1 });
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
  await draft().loadNewer();
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

test("scheduled dependencies accept completed archives without making archived agents Done or Needs you", async () => {
  const { dependencyDone, isDone, needsYou } = await import("../src/lib/sessionStatus.ts");
  const session = { status: "waiting", archived_done: false, seen_at: 0, status_since: 1, open_question: false };
  assert.equal(dependencyDone(session), true);
  assert.equal(dependencyDone({ ...session, imported_from: "hf://buckets/demo/traces/run" }), false);
  assert.equal(dependencyDone({ ...session, imported_from: "hf://buckets/demo/traces/run", archived_done: true }), true);
  session.status = "closed";
  assert.equal(dependencyDone(session), false);
  session.archived_done = true;
  assert.equal(dependencyDone(session), true);
  assert.equal(isDone(session), false);
  assert.equal(needsYou(session), false);
  for (const status of ["working", "stopped", "error", "interrupted", "quota_paused", "in_terminal"]) {
    assert.equal(dependencyDone({ ...session, status }), false);
  }
});

test("permission defaults are YOLO and remain independent for each harness and profile", async () => {
  const { rememberedRun } = await import("../src/stores/composer.ts");
  assert.equal(emptySpec().permission_mode, "yolo");
  useComposer.setState({ spec: { ...emptySpec(), harness: "claude" }, open: true });
  draft().update((s) => ({ ...s, permission_mode: "approvals" }));
  assert.equal(rememberedRun("claude", null).permission_mode, "approvals");
  assert.equal(rememberedRun("codex", null).permission_mode, "yolo");
  assert.equal(rememberedRun("claude", "other-account").permission_mode, "yolo");
  draft().update((s) => ({ ...s, target: { kind: "temp" } }));
  assert.equal(draft().spec.permission_mode, "approvals");
  draft().discard();
  assert.equal(draft().spec.permission_mode, "approvals");
  draft().rememberLaunch({ ...emptySpec(), harness: "codex", profile: "review", permission_mode: "approvals" }, 100);
  assert.equal(rememberedRun("codex", "review").permission_mode, "approvals");
  assert.equal(rememberedRun("codex", null).permission_mode, "yolo");
  // A more recent launch from another device supersedes an older browser selection.
  draft().rememberLaunch({ ...emptySpec(), harness: "claude", permission_mode: "yolo" }, Date.now() + 1);
  assert.equal(rememberedRun("claude", null).permission_mode, "yolo");
});

test("AI auto approval is remembered per profile and survives new drafts", async () => {
  const { rememberedRun } = await import("../src/stores/composer.ts");
  useComposer.setState({ spec: { ...emptySpec(), harness: "codex", profile: "ai-review" }, open: true });
  draft().update((s) => ({ ...s, permission_mode: "auto" }));
  assert.equal(rememberedRun("codex", "ai-review").permission_mode, "auto");
  assert.equal(rememberedRun("codex", null).permission_mode, "yolo");
  draft().discard();
  assert.equal(draft().spec.permission_mode, "auto");
  draft().rememberLaunch({ ...emptySpec(), harness: "claude", profile: "ai-review", permission_mode: "auto" }, Date.now() + 10);
  draft().start();
  assert.equal(draft().spec.permission_mode, "auto");
});

test("existing worktree choices require a reusable primary folder and account for every linked agent", async () => {
  const { reusableWorktrees } = await import("../src/features/launch/worktreeChoices.ts");
  const source = { id: "original", cwd: "/repo/task", status: "waiting", launch_spec: { target: { kind: "project" } } };
  const info = { path: source.cwd, exists: true, worktree: { id: "task", branch: "lp/task", project_id: "project", primary: true, state: "active", merged: null, sessions: [source.id, "followup"] } };
  const choices = (entry = info, sessions = [source]) => reusableWorktrees([entry], sessions, "project");
  assert.equal(choices()[0].source.id, source.id);
  assert.equal(choices()[0].busy, false);
  for (const status of ["starting", "working", "background", "in_terminal", "quota_paused"]) {
    assert.equal(choices(info, [source, { ...source, id: "followup", status }])[0].busy, true);
  }
  assert.deepEqual(reusableWorktrees([info], [source], "other-project"), []);
  assert.deepEqual(choices({ ...info, exists: false }), []);
  for (const patch of [{ primary: false }, { state: "removed" }, { state: "discarded" }, { state: "predraft" }, { state: "handed_off" }, { merged: {} }]) {
    assert.deepEqual(choices({ ...info, worktree: { ...info.worktree, ...patch } }), []);
  }
  assert.deepEqual(choices(info, [{ ...source, cwd: "/repo" }]), []);
  assert.deepEqual(choices(info, [{ ...source, launch_spec: { target: { kind: "browser" } } }]), []);
});
