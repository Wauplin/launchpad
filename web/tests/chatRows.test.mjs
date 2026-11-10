import assert from "node:assert/strict";
import { test } from "node:test";
import { chatRows, stepsLabel } from "../src/features/chat/chatRows.ts";

let n = 0;
const user = (text) => ({ key: `e${++n}`, v: "", t: "user", text, images: [], queued: false });
const answer = (text, streaming = false) => ({ key: `e${++n}`, v: "", t: "assistant", text, streaming });
const tools = (...names) => ({ key: `e${++n}`, v: "", t: "tools", calls: names.map((name, i) => ({ id: `c${n}-${i}`, revision: 0, name, summary: "launchpad inbox list", input: null, ts: 0 })) });
const end = (ts, extra = {}) => ({ key: `e${++n}`, v: "", t: "turn", ts, interrupted: false, error: null, duration_ms: 41_000, ...extra });
const session = (id, rows, extra = {}) => ({ id, started_at: 0, profile: "max", folder: null, running: false, rows, firstTs: 0, partial: false, changes: false, ...extra });
const kinds = (items) => items.map((i) => i.t);

test("a turn keeps the message, the last answer and one steps line; earlier messages become steps", () => {
  const items = chatRows([session("a", [user("What needs me today?"), answer("Checking the proposals too."), tools("Bash", "Bash"), answer("Two things need you."), end(100)])]);
  assert.deepEqual(kinds(items), ["divider", "user", "answer", "steps"]);
  assert.equal(items[2].text, "Two things need you.");
  assert.equal(items[3].count, 3);
  assert.deepEqual(items[3].steps, ["Checking the proposals too.", "Bash launchpad inbox list", "Bash launchpad inbox list"]);
  assert.equal(stepsLabel(items[3].count, items[3].durationMs), "3 steps · 41 s");
});

test("cards go to the turn that wrote them, and a quiet routine run is one line", () => {
  const routine = "Routine “Morning brief” · Scheduled\n\nRead the inbox.\n\nThis is a background run of the routine “Morning brief” (Scheduled). Nobody is watching it.";
  const rows = [user("first"), answer("one"), end(100), user(routine), tools("Bash"), answer("`no_update`"), end(200), user("second"), answer("two"), end(300)];
  const cards = [
    { kind: "proposal", id: "p2", session_id: "a", at: 250, v: "" },
    { kind: "proposal", id: "p1", session_id: "a", at: 50, v: "" },
    { kind: "changed", id: "k", session_id: "other", at: 60, v: "" },
  ];
  const items = chatRows([session("a", rows)], cards);
  assert.deepEqual(kinds(items), ["divider", "user", "answer", "card", "quiet", "user", "answer", "card"]);
  assert.equal(items[3].id, "p1");
  assert.equal(items[4].title, "Morning brief");
  assert.equal(items[7].id, "p2");
});

test("a running turn shows the live line instead of steps; a failed turn says so", () => {
  const items = chatRows([
    session("a", [user("hello"), answer("hi"), end(10, { error: "overloaded" })]),
    session("b", [user("next"), answer("Looking."), tools("Bash")], { running: true, profile: "pro", firstTs: 20 }),
  ]);
  assert.deepEqual(kinds(items), ["divider", "user", "answer", "ended", "divider", "user", "working"]);
  assert.equal(items[4].index, 2);
  assert.equal(items[4].reason, "the profile changed");
  assert.equal(items[6].step, "Bash launchpad inbox list");
});

test("cards written before the loaded events of a session wait until they load", () => {
  const items = chatRows([session("a", [user("x"), answer("y"), end(500)], { partial: true, firstTs: 400 })], [{ kind: "proposal", id: "old", session_id: "a", at: 100, v: "" }, { kind: "ready", id: "new", session_id: "a", at: 450, v: "" }]);
  assert.deepEqual(items.filter((i) => i.t === "card").map((i) => i.id), ["new"]);
});

test("a turn cut by the next message takes no cards, and keys stay unique across sessions", () => {
  const rows = [user("first"), answer("one"), user("second"), answer("two"), end(300)];
  const items = chatRows([session("a", rows), session("b", rows)], [{ kind: "proposal", id: "p", session_id: "a", at: 250, v: "" }]);
  assert.deepEqual(kinds(items).slice(0, 6), ["divider", "user", "answer", "user", "answer", "card"]);
  assert.equal(new Set(items.map((i) => i.key)).size, items.length);
});

test("a session divider names the recorded reason, and a message keeps its typed text without the new BOT.md", () => {
  const block = "Your instructions changed. The owner edited BOT.md since this session started or last heard of it. From now on, follow this version instead of the earlier one:\n\n<bot-instructions>\n# Scout\n</bot-instructions>";
  const items = chatRows([
    session("a", [user("hello"), answer("hi"), end(10)]),
    session("b", [user(`Check CI\n\n${block}\n\nContext:\n- Issue #3`), answer("ok"), end(20)], { reason: "model", firstTs: 15 }),
    session("c", [user(block), end(30)], { reason: "first", firstTs: 25 }),
  ]);
  const dividers = items.filter((i) => i.t === "divider");
  assert.deepEqual(dividers.map((d) => d.reason), [null, "the model changed", null]);
  const users = items.filter((i) => i.t === "user");
  assert.deepEqual(users.map((u) => [u.text, u.instructions]), [["hello", false], ["Check CI\n\nContext:\n- Issue #3", true], ["", true]]);
});
