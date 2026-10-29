import assert from "node:assert/strict";
import { test, after } from "node:test";
import { registerHooks } from "node:module";

registerHooks({ resolve(specifier, context, next) {
  if (specifier === "./ws" && context.parentURL?.endsWith("/reviewState.ts")) return { url: "data:text/javascript,export const onEvent = () => () => {};", shortCircuit: true };
  if (specifier.startsWith(".") && !/\.[a-z]+$/.test(specifier)) return next(`${specifier}.ts`, context);
  return next(specifier, context);
} });
const storage = () => {
  const map = new Map();
  return { get length() { return map.size; }, key: (i) => [...map.keys()][i] ?? null, getItem: (k) => map.get(k) ?? null, setItem: (k, v) => map.set(k, v), removeItem: (k) => map.delete(k) };
};
globalThis.localStorage = storage();
globalThis.sessionStorage = storage();
globalThis.location = { search: "", replace: () => {} };
const listeners = new Map();
globalThis.window = { addEventListener: (type, fn) => { listeners.set(type, [...(listeners.get(type) ?? []), fn]); } };
window.localStorage = localStorage;
window.sessionStorage = sessionStorage;
globalThis.setInterval = () => 0;
const event = (type, payload) => { for (const fn of listeners.get(type) ?? []) fn(payload); };
const tick = () => new Promise((resolve) => setImmediate(resolve));
const { api, ApiError } = await import("../src/lib/api.ts");
const realGet = api.get;
const { QueryClient } = await import("@tanstack/react-query");
const { scheduleReviewState, flushReviewState } = await import("../src/lib/reviewState.ts");
const { useSaveStatus } = await import("../src/lib/saveStatus.ts");
const qc = new QueryClient({ defaultOptions: { queries: { gcTime: Infinity } } });

after(() => qc.clear());

test("review saves survive failure, serialize edits and never install an old response", async () => {
  const key = "reply:repo#1";
  qc.setQueryData(["review-state", key], { key, value: "saved", updated_at: 10 });
  api.put = async () => { throw new Error("offline"); };
  scheduleReviewState(qc, key, "first", false);
  await flushReviewState(qc, key);
  assert.match(useSaveStatus.getState().errors[key].text, /not saved/);
  const backupKey = [...Array(localStorage.length)].map((_, i) => localStorage.key(i)).find((k) => k.startsWith("launchpad-review-pending:"));
  assert.equal(JSON.parse(localStorage.getItem(backupKey))[key].value, "first");
  const calls = [];
  let release;
  api.put = async (_, body) => {
    calls.push(body);
    if (calls.length === 1) await new Promise((resolve) => { release = resolve; });
    return { updated_at: 10 + calls.length };
  };
  const saving = flushReviewState(qc, key);
  scheduleReviewState(qc, key, "newest", false);
  await flushReviewState(qc, key);
  assert.equal(calls.length, 1);
  assert.equal(qc.getQueryData(["review-state", key]).value, "saved");
  release();
  await saving;
  assert.deepEqual(calls.map((c) => [c.value, c.base_revision]), [["first", 10], ["newest", 11]]);
  assert.equal(qc.getQueryData(["review-state", key]).value, "newest");
  assert.equal(localStorage.getItem(backupKey), null);
});

test("review conflicts preserve local text until an explicit choice", async () => {
  const key = "reply:conflict";
  api.put = async () => { throw new ApiError(409, "changed"); };
  scheduleReviewState(qc, key, "mine", false);
  await flushReviewState(qc, key);
  assert.equal(useSaveStatus.getState().errors[key].actions.length, 2);
  api.get = async () => ({ key, value: "theirs", updated_at: 100 });
  let sent;
  api.put = async (_, body) => { sent = body; return { updated_at: 101 }; };
  await useSaveStatus.getState().errors[key].actions.find((a) => a.label === "Keep mine").run();
  assert.equal(sent.value, "mine");
  assert.equal(sent.base_revision, 100);
  assert.equal(qc.getQueryData(["review-state", key]).value, "mine");
});

test("failed composer saves retain a backup and retry the latest revision on reconnect", async () => {
  const { useComposer } = await import("../src/stores/composer.ts");
  const st = () => useComposer.getState();
  api.put = async () => { throw new Error("offline"); };
  st().start();
  st().update((s) => ({ ...s, text: "keep this prompt" }));
  event("pagehide");
  await tick();
  assert.match(useSaveStatus.getState().errors["composer:null"].text, /not saved/);
  const backupKey = [...Array(localStorage.length)].map((_, i) => localStorage.key(i)).find((k) => k.startsWith("launchpad-composer-pending:"));
  assert.equal(JSON.parse(localStorage.getItem(backupKey))[0].body.spec.text, "keep this prompt");
  api.get = async (path) => path === "/api/sessions" || path === "/api/projects" ? [] : { spec: { text: "old server version" }, revision: 0 };
  // A reload/restore keeps the unsent copy, rather than installing the old server version.
  api.get = async () => { throw new Error("offline"); };
  await st().restore();
  assert.equal(st().spec.text, "keep this prompt");
  await tick();
  let sent;
  api.put = async (_, body) => { sent = body; return { revision: 1 }; };
  event("launchpad-reconnected");
  await tick();
  assert.equal(sent.spec.text, "keep this prompt");
  assert.equal(st().revision, 1);
  assert.equal(localStorage.getItem(backupKey), null);
});

test("message and layout writes include changes from another window", async () => {
  const { useMessageDrafts } = await import("../src/stores/messages.ts");
  localStorage.setItem("launchpad-message-drafts", JSON.stringify({ state: { drafts: { other: "unsent elsewhere" } }, version: 0 }));
  useMessageDrafts.getState().set("here", "local text");
  assert.deepEqual(useMessageDrafts.getState().drafts, { other: "unsent elsewhere", here: "local text" });
  const context = { context_projects: [{ project_id: "reference", branch: null, editable: false }], github_refs: [], agent_traces: [{ session_id: "source", through_seq: 12 }] };
  useMessageDrafts.getState().setContext("here", context);
  useMessageDrafts.getState().set("here", "updated text");
  assert.deepEqual(JSON.parse(localStorage.getItem("launchpad-message-drafts")).state.contexts.here, context);
  // Another window's unsent context survives clearing this session after delivery.
  const storedMessages = JSON.parse(localStorage.getItem("launchpad-message-drafts"));
  storedMessages.state.contexts.other = context;
  localStorage.setItem("launchpad-message-drafts", JSON.stringify(storedMessages));
  useMessageDrafts.getState().setContext("here", { context_projects: [], github_refs: [], agent_traces: [] });
  assert.deepEqual(useMessageDrafts.getState().contexts, { other: context });
  event("storage", { key: "launchpad-message-drafts" });
  const { useLayout } = await import("../src/stores/layout.ts");
  localStorage.setItem("launchpad.layout", JSON.stringify({ state: { pinned: ["other"] }, version: 0 }));
  useLayout.getState().setDock({ open: true });
  assert.deepEqual(useLayout.getState().pinned, ["other"]);
  useLayout.getState().togglePin("here");
  assert.deepEqual(useLayout.getState().pinned, ["other", "here"]);
});

test("composer retries keep the original workspace revision after switching", async () => {
  const { useComposer } = await import("../src/stores/composer.ts");
  const { useWorkspace } = await import("../src/stores/workspace.ts");
  api.get = async (path) => path === "/api/sessions" || path === "/api/projects" ? [] : { revision: 20 };
  api.put = async () => { throw new Error("offline"); };
  useComposer.getState().update((spec) => ({ ...spec, text: "workspace A" }));
  useWorkspace.getState().switchTo("other");
  await tick();
  assert.equal(useComposer.getState().revision, 20);
  const calls = [];
  api.put = async (path, body) => { calls.push([path, body.base_revision]); return { revision: 2 }; };
  event("launchpad-reconnected");
  await tick();
  assert.deepEqual(calls, [["/api/launch/draft", 1]]);
  assert.equal(useComposer.getState().revision, 20);
  useWorkspace.getState().switchTo(null);
  await tick();
});

test("a lost forced composer save retries conditionally instead of overwriting another edit", async () => {
  const { useComposer } = await import("../src/stores/composer.ts");
  let first;
  api.put = async (_, body) => { first = body; throw new Error("lost response"); };
  useComposer.getState().keepMine();
  await tick();
  assert.equal(first.force, true);
  let retried;
  api.put = async (_, body) => { retried = body; throw new ApiError(409, "changed"); };
  event("launchpad-reconnected");
  await tick();
  assert.equal(retried.force, false);
  assert.equal(useComposer.getState().conflict, true);
  await useComposer.getState().loadNewer();
});

test("sign out erases cached data and disables background writers before redirect", async () => {
  const { storageAllowed } = await import("../src/lib/clientStorage.ts");
  let cleared = false;
  let closed = false;
  let redirected = false;
  globalThis.indexedDB = { open: () => {
    const req = {};
    queueMicrotask(() => {
      req.result = {
        transaction: () => {
          const tx = { objectStore: () => ({ clear: () => { cleared = true; } }) };
          queueMicrotask(() => tx.oncomplete());
          return tx;
        },
        close: () => { closed = true; },
      };
      req.onsuccess();
    });
    return req;
  } };
  location.replace = (path) => { assert.equal(path, "/auth"); assert.equal(cleared, true); redirected = true; };
  localStorage.setItem("unrelated", "keep");
  localStorage.setItem("launchpad-message-drafts", "sensitive");
  localStorage.setItem("github-reply:repo", "sensitive");
  sessionStorage.setItem("launchpad-composer-client", "sensitive");
  globalThis.fetch = async () => new Response(JSON.stringify({ error: "Revoked" }), { status: 401 });
  await assert.rejects(realGet("/api/status"), (e) => e.status === 401);
  assert.equal(storageAllowed(), false);
  assert.equal(localStorage.length, 1);
  assert.equal(localStorage.getItem("unrelated"), "keep");
  assert.equal(sessionStorage.length, 0);
  assert.equal(closed, true);
  assert.equal(redirected, true);
  await assert.rejects(realGet("/api/status"), (e) => e.status === 401);
});
