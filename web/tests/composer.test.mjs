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
const merge = () => { const w = draft().spec.worktree; return w ? w.when_done === "merge" : undefined; };

test("new drafts and project switches use separate defaults; context additions preserve unsent choices", () => {
  rememberProjectDefaults([{ id: "review", when_done: "keep" }, { id: "merge", when_done: "merge" }]);
  useComposer.setState({ spec: emptySpec(), open: false });
  draft().start({ projectId: "merge" });
  assert.equal(merge(), true);
  draft().update((s) => ({ ...s, target: { kind: "project", project_id: "review" } }));
  assert.equal(merge(), false);
  draft().update((s) => ({ ...s, target: { kind: "project", project_id: "merge" } }));
  assert.equal(merge(), true);
  draft().update((s) => ({ ...s, worktree: { ...s.worktree, when_done: "keep" } }));
  draft().start({ projectId: "merge", githubRefs: [] });
  assert.equal(merge(), false);
  rememberProjectCompletion("merge", "keep");
  draft().discard();
  draft().start({ projectId: "merge" });
  assert.equal(merge(), false);
  draft().start({ projectId: "unknown" });
  assert.equal(merge(), false);
});

test("restored drafts keep their choice, and newly enabled worktrees use the project default", async () => {
  const saved = { ...emptySpec(), target: { kind: "project", project_id: "merge" } };
  api.get = async (path) => path === "/api/projects" ? [{ id: "merge", when_done: "merge" }] : path === "/api/sessions" ? [] : { spec: saved, open: true };
  await draft().loadNewer();
  await draft().restore();
  assert.equal(merge(), false);
  draft().start();
  assert.equal(merge(), false);
  draft().update((s) => ({ ...s, worktree: null }));
  draft().update((s) => ({ ...s, worktree: emptySpec().worktree }));
  assert.equal(merge(), true);
  draft().update((s) => ({ ...s, target: { kind: "predraft", project_id: "merge" }, worktree: { ...s.worktree, when_done: "keep" } }));
  assert.equal(merge(), false);
});

test("scheduled dependencies accept completed archives without making archived agents Done or Needs you", async () => {
  const { dependencyDone, isDone, needsYou } = await import("../src/lib/sessionStatus.ts");
  const session = { status: "waiting", archived_done: false, seen_at: 0, status_since: 1, open_question: false };
  assert.equal(dependencyDone(session), true);
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
});


test("preset availability excludes stale selections while preserving settings order", async () => {
  const { availablePresets, withPresets } = await import("../src/lib/presets.ts");
  const presets = [
    { id: "ask", text: "Implement with Claude, then review with Codex.", bots: true, agent: false },
    { id: "both", text: "Keep it concise.", bots: true, agent: true },
    { id: "agent", text: "Run the checks.", bots: false, agent: true },
    { id: "off", text: "Hidden.", bots: false, agent: false },
    { id: "legacy", text: "Existing preset." },
  ];
  const selected = ["legacy", "off", "agent", "both", "ask", "deleted"];
  assert.equal(withPresets("Fix it", selected, availablePresets(presets, "bots")), "Fix it\n\nImplement with Claude, then review with Codex. Keep it concise.");
  assert.equal(withPresets("Fix it", selected, availablePresets(presets, "agent")), "Fix it\n\nKeep it concise. Run the checks. Existing preset.");
  assert.equal(withPresets("", ["ask"], availablePresets(presets, "bots")), presets[0].text);
  presets[0].bots = false;
  assert.equal(withPresets("", ["ask"], availablePresets(presets, "bots")), "");
});

const { prepareLaunch, launchBlocker, withFolder } = await import("../src/features/launch/launchDecisions.ts");
test("submission preserves launch options and only normalizes workspace, worktree and PR", () => {
  const project = { id: "repo", gh_repo: "owner/repo", home_git: true, when_done: "keep" };
  const original = { ...emptySpec(), text: "Inspect https://github.com/owner/repo/pull/12", attachments: [{ name: "notes.txt", data_url: "data:text/plain;base64,YQ==" }], profile: "chosen", model: "chosen-model", effort: "high", target: { kind: "project", project_id: "repo" } };
  const snapshot = structuredClone(original);
  const result = prepareLaunch(original, "work", true, project, null);
  assert.deepEqual(result, { ...original, workspace_id: "work", worktree: { ...original.worktree, pull_request: { repo: "owner/repo", number: 12 } } });
  assert.deepEqual(original, snapshot);
  assert.equal(prepareLaunch(original, "work", true, project, "owner/repo#12").worktree.pull_request, undefined);
  assert.equal(prepareLaunch(original, "work", false, project, null).worktree, null);
  for (const target of [{ kind: "temp" }, { kind: "worktree", worktree_id: "existing" }, { kind: "predraft", project_id: "repo" }]) {
    const draft = { ...original, target };
    assert.deepEqual(prepareLaunch(draft, "", true, project, null), { ...draft, workspace_id: null, worktree: { ...draft.worktree, pull_request: undefined } });
  }
  assert.deepEqual(withFolder({ ...original, context_projects: [{ project_id: "repo" }, { project_id: "other" }] }, project).context_projects, [{ project_id: "other" }]);
});

test("launch validation keeps blocker precedence and timing checks", () => {
  const valid = { spec: { ...emptySpec(), harness: "codex", profile: "chosen", target: { kind: "temp" } }, hasProfiles: true, workspace: "work", hasPrompt: true, budgetValid: true, timingValid: true };
  assert.equal(launchBlocker(valid), null);
  assert.equal(launchBlocker({ ...valid, editGone: true, busyWorktree: true }), "This scheduled agent already started or was cancelled");
  assert.equal(launchBlocker({ ...valid, editing: true, timing: "later", scheduled: false }), "Add a start condition, or choose Start now");
  assert.equal(launchBlocker({ ...valid, hasPrompt: false, budgetValid: false }), "Write a prompt");
  assert.equal(launchBlocker({ ...valid, timingValid: false }), "Fix the start or repeat options");
});

test("shared thread identity preserves missing, empty and explicit thread IDs", async () => {
  const { launchThread, threadOf } = await import("../src/lib/botThread.ts");
  for (const thread of [undefined, null, "", "main", "github:owner/repo#12"]) {
    assert.equal(launchThread({ thread }), thread || "main");
    assert.equal(threadOf({ launch_spec: { bot: { thread } } }), thread || "main");
  }
  assert.equal(threadOf({ launch_spec: {} }), "main");
});


test("thread selection retains the newest session, including archives and tied timestamps", async () => {
  const { threadCurrents } = await import("../src/lib/botThread.ts");
  const sessions = [
    { id: "new-main", started_at: 3, status: "working", launch_spec: {} },
    { id: "old-main", started_at: 1, status: "waiting", launch_spec: { bot: { thread: "" } } },
    { id: "old-topic", started_at: 2, status: "waiting", launch_spec: { bot: { thread: "topic" } } },
    { id: "closed-topic", started_at: 4, status: "closed", launch_spec: { bot: { thread: "topic" } } },
    { id: "tie-first", started_at: 5, status: "waiting", launch_spec: { bot: { thread: "tie" } } },
    { id: "tie-last", started_at: 5, status: "working", launch_spec: { bot: { thread: "tie" } } },
  ];
  const before = structuredClone(sessions);
  const currents = threadCurrents(sessions);
  assert.deepEqual([...currents].map(([id, s]) => [id, s.id]), [["main", "new-main"], ["tie", "tie-last"]]);
  assert.deepEqual(sessions, before);
});
