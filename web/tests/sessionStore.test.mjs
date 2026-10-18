import assert from "node:assert/strict";
import { test } from "node:test";
import { useSessionLogs, emptyLog, reloadSessionLogs } from "../src/lib/sessionStore.ts";
import { api } from "../src/lib/api.ts";
import { QueryClient } from "@tanstack/react-query";
import { restoreCache } from "../src/lib/offlineCache.ts";

const event = (seq) => ({ session_id: "a", seq, ts: seq, body: { kind: "assistant_message", item_id: `m${seq}`, text: `message ${seq}` } });
const deferred = () => { let resolve; const promise = new Promise((r) => { resolve = r; }); return { promise, resolve }; };
const reset = (log = emptyLog()) => useSessionLogs.setState({ logs: { a: log } });

test("a final event during the tail fetch triggers catch-up and settles streamed text", async () => {
  reset();
  const original = api.get;
  const pending = deferred();
  const paths = [];
  api.get = async (path) => { paths.push(path); return paths.length === 1 ? pending.promise : [event(301)]; };
  try {
    const load = useSessionLogs.getState().load("a");
    useSessionLogs.getState().receive({ ...event(0), body: { kind: "assistant_delta", item_id: "m301", text: "stream" } });
    useSessionLogs.getState().receive(event(301));
    pending.resolve({ events: [event(300)], facts: null });
    await load;
    const log = useSessionLogs.getState().logs.a;
    assert.deepEqual(log.events.map((e) => e.seq), [300, 301]);
    assert.equal(log.lastSeq, 301);
    assert.deepEqual(log.deltas, {});
    assert.ok(paths[1].endsWith("after=300"));
    assert.equal(log.error, null);
  } finally { api.get = original; }
});

test("a gap arriving during catch-up is fetched again, without duplicate events", async () => {
  reset({ ...emptyLog(), events: [event(1)], lastSeq: 1, loaded: true });
  const original = api.get;
  const pending = deferred();
  let calls = 0;
  api.get = async () => ++calls === 1 ? pending.promise : [event(3), event(4)];
  try {
    const load = useSessionLogs.getState().load("a");
    useSessionLogs.getState().receive(event(2));
    useSessionLogs.getState().receive(event(4));
    pending.resolve([event(2), event(3)]);
    await load;
    assert.deepEqual(useSessionLogs.getState().logs.a.events.map((e) => e.seq), [1, 2, 3, 4]);
    assert.equal(calls, 2);
  } finally { api.get = original; }
});

test("first fetch failure is visible and manual retry restores the tail even with live events", async () => {
  reset();
  const original = api.get;
  api.get = async () => { throw Error("offline"); };
  try {
    await useSessionLogs.getState().load("a");
    assert.equal(useSessionLogs.getState().logs.a.error, "offline");
    assert.equal(useSessionLogs.getState().logs.a.loading, false);
    useSessionLogs.getState().receive(event(1));
    api.get = async (path) => {
      assert.ok(path.includes("tail="));
      return { events: [event(1), event(2)], facts: null };
    };
    await useSessionLogs.getState().load("a");
    assert.equal(useSessionLogs.getState().logs.a.error, null);
    assert.equal(useSessionLogs.getState().logs.a.loaded, true);
    assert.deepEqual(useSessionLogs.getState().logs.a.events.map((e) => e.seq), [1, 2]);
  } finally { api.get = original; }
});

test("replayed reconnect retries incomplete logs but leaves healthy logs alone", async () => {
  reset();
  useSessionLogs.setState((s) => ({ logs: { ...s.logs, healthy: { ...emptyLog(), loaded: true } } }));
  const original = api.get;
  const done = deferred();
  const paths = [];
  api.get = async (path) => { paths.push(path); done.resolve(); return { events: [event(1)], facts: null }; };
  try {
    reloadSessionLogs(true);
    await done.promise;
    // Let the load's continuation settle.
    await Promise.resolve();
    assert.equal(paths.length, 1);
    assert.ok(paths[0].includes("/a/"));
    assert.equal(useSessionLogs.getState().logs.a.loaded, true);
  } finally { api.get = original; }
});

test("delayed offline restore cannot replace fresh daemon data; restored data can be invalidated", async () => {
  const old = globalThis.indexedDB;
  const opened = deferred();
  const snapshot = { at: 1, entries: [["sessions", ["old"]], ["projects", ["cached"]]] };
  globalThis.indexedDB = { open: () => {
    const request = {};
    opened.resolve(request);
    return request;
  } };
  const qc = new QueryClient({ defaultOptions: { queries: { staleTime: Infinity } } });
  try {
    const restoring = restoreCache(qc);
    const request = await opened.promise;
    qc.setQueryData(["sessions"], ["fresh"]);
    request.result = { transaction: () => ({ objectStore: () => ({ get: () => {
      const read = { result: snapshot };
      queueMicrotask(() => read.onsuccess());
      return read;
    } }) }) };
    request.onsuccess();
    await restoring;
    assert.deepEqual(qc.getQueryData(["sessions"]), ["fresh"]);
    assert.deepEqual(qc.getQueryData(["projects"]), ["cached"]);
    await qc.invalidateQueries();
    assert.equal(qc.getQueryState(["projects"]).isInvalidated, true);
  } finally { globalThis.indexedDB = old; qc.clear(); }
});
