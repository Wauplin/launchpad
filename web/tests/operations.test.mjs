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

test("HTTP crypto fallback sends UUID keys, retries lost replies and matches secure fingerprints", async () => {
  const { api } = await import("../src/lib/api.ts");
  const nativeCrypto = globalThis.crypto;
  const oldFetch = globalThis.fetch;
  const oldStorage = globalThis.localStorage;
  const saved = storage();
  globalThis.localStorage = saved;
  const path = "/api/sessions/a/input", body = { text: "HTTP message 🎉" };
  const secure = await operationKey(path, body, saved);
  const keys = [];
  Object.defineProperty(globalThis, "crypto", { configurable: true, value: { getRandomValues: (a) => nativeCrypto.getRandomValues(a) } });
  globalThis.fetch = async (_path, options) => {
    keys.push(options.headers["Idempotency-Key"]);
    if (keys.length === 2) throw new TypeError("lost reply");
    return new Response('{}', { headers: { "Idempotency-Status": "completed" } });
  };
  try {
    assert.equal((await operationKey(path, body, saved)).id, secure.id);
    await api.post("/api/sessions", body);
    await assert.rejects(api.post(path, body), /lost reply/);
    await api.post(path, body);
    assert.equal(keys[1], keys[2]);
    assert.match(keys[0], /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
    assert.equal(saved.values.size, 0);
  } finally {
    Object.defineProperty(globalThis, "crypto", { configurable: true, value: nativeCrypto });
    globalThis.fetch = oldFetch; globalThis.localStorage = oldStorage;
  }
});

test("fallback SHA-256 agrees with native hashing across padding boundaries and attachment sizes", async () => {
  const { sha256 } = await import("../src/lib/operationHash.ts");
  const { createHash } = await import("node:crypto");
  for (const size of [0, 3, 55, 56, 63, 64, 65, 119, 120, 128, 1024, 1024 * 1024]) {
    const bytes = Uint8Array.from({ length: size }, (_, i) => i % 251);
    assert.equal(sha256(bytes), createHash("sha256").update(bytes).digest("hex"), `size ${size}`);
  }
});

test("expired attempts block fetch until explicit abandonment, then get a new ID", async () => {
  const { OPERATION_RETRY_MS, abandonOperation } = await import("../src/lib/operations.ts");
  const { api } = await import("../src/lib/api.ts");
  const oldNow = Date.now, oldStorage = globalThis.localStorage, oldFetch = globalThis.fetch;
  const saved = storage();
  globalThis.localStorage = saved;
  const path = "/api/sessions/a/input", body = { text: "yes" };
  let now = oldNow();
  Date.now = () => now;
  let fetches = 0;
  globalThis.fetch = async () => { fetches++; return new Response('{}', { headers: { "Idempotency-Status": "completed" } }); };
  try {
    const first = await operationKey(path, body, saved);
    now += OPERATION_RETRY_MS - 1;
    assert.equal((await operationKey(path, body, saved)).id, first.id);
    now++;
    await assert.rejects(api.post(path, body), /too old/);
    assert.equal(fetches, 0);
    assert.equal(saved.values.size, 1);
    await abandonOperation(path, body, saved);
    const next = await operationKey(path, body, saved);
    assert.notEqual(next.id, first.id);
    first.delivered(); // A late response must not erase the replacement operation.
    assert.equal((await operationKey(path, body, saved)).id, next.id);
    await api.post(path, body);
    assert.equal(fetches, 1);
  } finally { Date.now = oldNow; globalThis.localStorage = oldStorage; globalThis.fetch = oldFetch; }
});

test("legacy and malformed ledger entries need review and can be abandoned without generating an ID", async () => {
  const { abandonOperation } = await import("../src/lib/operations.ts");
  const saved = storage(), path = "/api/sessions", body = {};
  await operationKey(path, body, saved);
  const key = [...saved.values.keys()][0];
  for (const value of ["old-uuid", "null", "{}", '{"id":"old","createdAt":1}']) {
    saved.setItem(key, value);
    await assert.rejects(operationKey(path, body, saved), /saved attempt/);
    await abandonOperation(path, body, saved);
    assert.equal(saved.values.size, 0);
  }
  await abandonOperation(path, body, saved);
  assert.equal(saved.values.size, 0);
});
