import assert from "node:assert/strict";
import { test } from "node:test";
import { activeThreads, capThreads, ACTIVE_WINDOW_MS } from "../src/lib/activeThreads.ts";

const NOW = 10 * ACTIVE_WINDOW_MS;
const HOUR = 3_600_000;
const bot = (id, workspace_id = "w") => ({ id, workspace_id, settings: { name: id.toUpperCase() } });
let n = 0;
// A bot session: Done and seen, updated `ago` ms before NOW, unless `extra` says otherwise.
const session = (botId, thread, ago, extra = {}) => ({ id: `s${++n}`, title: `session ${thread}`, status: "waiting", started_at: NOW - ago, status_since: NOW - ago, seen_at: NOW, open_question: null, launch_spec: { bot: { id: botId, thread } }, ...extra });
const ids = (rows) => rows.map((r) => `${r.bot.id}:${r.thread}`);

test("keeps live and recent threads, drops archived, old and other workspaces' ones", () => {
  const bots = [bot("a"), bot("b"), bot("x", "other")];
  const sessions = [
    session("a", "recent", 2 * HOUR),
    session("a", "old", 30 * HOUR),
    session("a", "working", 40 * HOUR, { status: "working" }),
    session("a", "sleeping", 40 * HOUR, { sleep: { waits: "later" } }),
    session("a", "failed", 40 * HOUR, { status: "error", capacity_retry_at: null }),
    session("a", "paused", 40 * HOUR, { status: "quota_paused" }),
    session("a", "question", 40 * HOUR, { open_question: { id: "q" } }),
    session("a", "unseen", 40 * HOUR, { seen_at: 0 }),
    session("b", "archived", HOUR, { status: "closed" }),
    session("x", "elsewhere", HOUR, { status: "working" }),
  ];
  const rows = activeThreads(bots, sessions, new Map(), "w", NOW);
  assert.deepEqual(ids(rows).sort(), ["a:failed", "a:paused", "a:question", "a:recent", "a:sleeping", "a:unseen", "a:working"]);
  const label = Object.fromEntries(rows.map((r) => [r.thread, r.label]));
  assert.equal(label.working, "Working");
  assert.equal(label.sleeping, "Sleeping");
  assert.equal(label.failed, "Failed");
  assert.equal(label.question, "Question");
  assert.equal(label.recent, "Done");
});

test("a thread counts only by its newest session, and Home's list names it or hides it as archived", () => {
  const bots = [bot("a")];
  const sessions = [
    session("a", "t1", 3 * HOUR, { status: "working", started_at: NOW - 5 * HOUR }),
    session("a", "t1", HOUR, { status: "closed", started_at: NOW - HOUR }),
    session("a", "t2", HOUR),
    session("a", "t3", HOUR),
  ];
  const threads = new Map([["a", [{ id: "t2", title: "Renamed" }]]]);
  const rows = activeThreads(bots, sessions, threads, "w", NOW);
  assert.deepEqual(ids(rows), ["a:t2"]);
  assert.equal(rows[0].title, "Renamed");
  // Before the list loads, the session's title stands in.
  assert.equal(activeThreads(bots, sessions, new Map(), "w", NOW).find((r) => r.thread === "t3").title, "session t3");
});

test("sorts by latest update first", () => {
  const rows = activeThreads([bot("a"), bot("b")], [session("a", "1", 3 * HOUR), session("b", "2", HOUR), session("a", "3", 2 * HOUR, { status: "working" })], new Map(), "w", NOW);
  assert.deepEqual(ids(rows), ["b:2", "a:3", "a:1"]);
});

test("the cap keeps threads that need attention before newer done ones, in order", () => {
  const sessions = [
    ...[1, 2, 3, 4, 5, 6, 7].map((i) => session("a", `done${i}`, i * 60_000)),
    session("a", "working", 10 * HOUR, { status: "working" }),
    session("a", "failed", 11 * HOUR, { status: "interrupted" }),
  ];
  const rows = activeThreads([bot("a")], sessions, new Map(), "w", NOW);
  assert.equal(rows.length, 9);
  assert.deepEqual(ids(capThreads(rows)), ["a:done1", "a:done2", "a:done3", "a:done4", "a:working", "a:failed"]);
  assert.equal(capThreads(rows.slice(0, 5)).length, 5);
  const busy = [1, 2, 3, 4, 5, 6, 7, 8].map((i) => session("a", `w${i}`, i * HOUR, { status: "working" }));
  assert.deepEqual(ids(capThreads(activeThreads([bot("a")], [...busy, session("a", "new", 1000)], new Map(), "w", NOW))), ["a:w1", "a:w2", "a:w3", "a:w4", "a:w5", "a:w6"]);
});

test("a conversation of several bots is one row: the main bot's thread, every bot's mark, the most urgent state", () => {
  const bots = [bot("a"), bot("b"), bot("c")];
  const sessions = [
    session("a", "t1", HOUR),
    session("b", "t9", 2 * HOUR, { status: "working" }),
    session("c", "t5", 3 * HOUR, { open_question: { id: "q" } }),
    session("b", "own", 4 * HOUR),
  ];
  const threads = new Map([
    ["a", [{ id: "t1", title: "Release", conversation: "a:t1" }]],
    ["b", [{ id: "t9", title: "Release", conversation: "a:t1" }, { id: "own", title: "Own", conversation: "b:own" }]],
    ["c", [{ id: "t5", title: "Release", conversation: "a:t1" }]],
  ]);
  const rows = activeThreads(bots, sessions, threads, "w", NOW);
  assert.deepEqual(ids(rows), ["a:t1", "b:own"]);
  assert.deepEqual(rows[0].bots.map((b) => b.id), ["a", "b", "c"]);
  assert.equal(rows[0].label, "Question");
  assert.equal(rows[0].attention, true);
  assert.equal(rows[0].last, NOW - HOUR);
  // Failed outranks Working, Working outranks Sleeping and Done.
  const two = activeThreads(bots, [session("a", "t1", HOUR, { status: "error", capacity_retry_at: null }), session("b", "t9", HOUR, { status: "working" })], threads, "w", NOW);
  assert.equal(two[0].label, "Failed");
  const three = activeThreads(bots, [session("a", "t1", HOUR, { sleep: { waits: "x" } }), session("b", "t9", 2 * HOUR, { status: "working" })], threads, "w", NOW);
  assert.equal(three[0].label, "Working");
  // Without the main bot's thread, the row opens the most urgent one.
  const member = activeThreads(bots, [session("b", "t9", HOUR), session("c", "t5", HOUR, { status: "working" })], threads, "w", NOW);
  assert.deepEqual(ids(member), ["c:t5"]);
});
