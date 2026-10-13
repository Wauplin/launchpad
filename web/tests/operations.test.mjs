import assert from "node:assert/strict";
import { test } from "node:test";
import { operationKey, retryProtected } from "../src/lib/operations.ts";

function storage() {
  const values = new Map();
  return { getItem: (k) => values.get(k) ?? null, setItem: (k, v) => values.set(k, v), removeItem: (k) => values.delete(k), values };
}

test("lost replies and recreated clients reuse the saved ID; delivery permits a new identical message", async () => {
  const saved = storage();
  const path = "/api/sessions/a/input";
  const first = await operationKey(path, { text: "hello" }, saved);
  const retry = await operationKey(path, { text: "hello" }, saved);
  assert.equal(retry.id, first.id);
  assert.equal(saved.values.size, 1);
  assert.ok(!JSON.stringify([...saved.values]).includes("hello"));
  retry.delivered();
  assert.notEqual((await operationKey(path, { text: "hello" }, saved)).id, first.id);
});

test("different messages, sessions and launch specs get independent IDs", async () => {
  const saved = storage();
  const requests = [["/api/sessions", { text: "hello" }], ["/api/sessions/a/input", { text: "hello" }], ["/api/sessions/b/input", { text: "hello" }], ["/api/sessions/a/input", { text: "changed" }]];
  const ids = await Promise.all(requests.map(async ([path, body]) => (await operationKey(path, body, saved)).id));
  assert.equal(new Set(ids).size, requests.length);
  const concurrent = await Promise.all([operationKey(...requests[0], saved), operationKey(...requests[0], saved)]);
  assert.equal(concurrent[0].id, concurrent[1].id);
});

test("unavailable durable storage fails before a request can be sent", async () => {
  await assert.rejects(operationKey("/api/sessions", {}, { getItem: () => null, setItem: () => { throw Error("storage full"); } }), /storage full/);
});

test("only immediate launch and session input use retry protection", () => {
  for (const p of ["/api/sessions", "/api/sessions/a/input"]) assert.equal(retryProtected(p), true);
  for (const p of ["/api/sessions/a/answer", "/api/launch/schedules", "/api/sessions/a/input/other"]) assert.equal(retryProtected(p), false);
});

test("API retries a lost reply with the same key and clears it only after a recorded response", async () => {
  const { api } = await import("../src/lib/api.ts");
  const oldFetch = globalThis.fetch;
  const oldStorage = globalThis.localStorage;
  globalThis.localStorage = storage();
  const keys = [];
  globalThis.fetch = async (_path, options) => {
    keys.push(options.headers["Idempotency-Key"]);
    if (keys.length === 1) throw new TypeError("connection lost after acceptance");
    return new Response('{"id":"first"}', { headers: { "Idempotency-Status": "completed" } });
  };
  try {
    await assert.rejects(api.post("/api/sessions", { text: "task" }), /connection lost/);
    assert.deepEqual(await api.post("/api/sessions", { text: "task" }), { id: "first" });
    assert.equal(keys[0], keys[1]);
    await api.post("/api/sessions", { text: "task" });
    assert.notEqual(keys[1], keys[2]);
  } finally { globalThis.fetch = oldFetch; globalThis.localStorage = oldStorage; }
});

test("uncertain responses retain the key; recorded failures permit an explicit new attempt", async () => {
  const { api, ApiError } = await import("../src/lib/api.ts");
  const oldFetch = globalThis.fetch;
  const oldStorage = globalThis.localStorage;
  globalThis.localStorage = storage();
  const keys = [];
  let completed = false;
  globalThis.fetch = async (_path, options) => {
    keys.push(options.headers["Idempotency-Key"]);
    return new Response('{"error":"not sent"}', { status: completed ? 400 : 409, headers: completed ? { "Idempotency-Status": "completed" } : {} });
  };
  try {
    await assert.rejects(api.post("/api/sessions/a/input", { text: "message" }), (e) => e instanceof ApiError && !e.operationCompleted);
    completed = true;
    await assert.rejects(api.post("/api/sessions/a/input", { text: "message" }), (e) => e.operationCompleted);
    assert.equal(keys[0], keys[1]);
    await assert.rejects(api.post("/api/sessions/a/input", { text: "message" }));
    assert.notEqual(keys[1], keys[2]);
  } finally { globalThis.fetch = oldFetch; globalThis.localStorage = oldStorage; }
});
