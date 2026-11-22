import assert from "node:assert/strict";
import { test } from "node:test";
import { completeMention, conversationStatus, mentionChoices, mentionQuery, mergeConversation, routeMentions } from "../src/lib/conversations.ts";

const BOTS = [
  { id: "js", slug: "hubert-js-bot", name: "Hubert.js Bot" },
  { id: "py", slug: "hubert-py-bot", name: "Hubert.py Bot" },
  { id: "ask", slug: "ask-launchpad", name: "Ask Launchpad" },
];

test("no mention: the message goes to the main bot", () => {
  assert.deepEqual(routeMentions("compare the release flows", BOTS, "ask"), { targets: ["ask"] });
});

test("one leading mention: that bot only", () => {
  assert.deepEqual(routeMentions("@hubert-js-bot what changed?", BOTS, "ask"), { targets: ["js"] });
});

test("several leading mentions: each of those bots, in order, once", () => {
  assert.deepEqual(routeMentions("@hubert-js-bot @hubert-py-bot compare your release flows", BOTS, "ask"), { targets: ["js", "py"] });
  assert.deepEqual(routeMentions("  @hubert-py-bot, @hubert-js-bot @hubert-py-bot go", BOTS, "ask"), { targets: ["py", "js"] });
});

test("a mention mid-text does not route", () => {
  assert.deepEqual(routeMentions("@hubert-js-bot please check with @hubert-py-bot", BOTS, "ask"), { targets: ["js"] });
  assert.deepEqual(routeMentions("ask @hubert-py-bot about it", BOTS, "ask"), { targets: ["ask"] });
});

test("an unknown name is refused before anything is sent", () => {
  assert.deepEqual(routeMentions("@nobody hi", BOTS, "ask"), { error: "@nobody: no bot of this workspace has that name" });
  assert.deepEqual(routeMentions("@hubert-js-bot @nobody hi", BOTS, "ask"), { error: "@nobody: no bot of this workspace has that name" });
  assert.ok("error" in routeMentions("@hubert-js-bot   ", BOTS, "ask"));
});

test("@ suggestions while typing the leading mentions, participants first", () => {
  assert.equal(mentionQuery("@hub"), "hub");
  assert.equal(mentionQuery("@hubert-js-bot @"), "");
  assert.equal(mentionQuery("hello @hub"), null);
  assert.equal(mentionQuery("@hubert-js-bot hello"), null);
  assert.deepEqual(mentionChoices("hub", BOTS, ["py"]).map((b) => b.id), ["py", "js"]);
  assert.equal(completeMention("@hubert-js-bot @hu", "@hubert-js-bot @hu", "hubert-py-bot"), "@hubert-js-bot @hubert-py-bot ");
});

test("the status line lists the most urgent bot first", () => {
  const s = (extra) => ({ status: "waiting", open_question: null, sleep: null, ...extra });
  assert.equal(conversationStatus([{ name: "A", session: s({}) }, { name: "B", session: s({ status: "working" }) }, { name: "C", session: s({ open_question: { id: "q" } }) }]), "C waits for you · B working · A waiting");
});

let n = 0;
const item = (t, extra = {}) => ({ key: `k${++n}`, v: "", t, ...extra });

test("the merged timeline orders each bot's turns by time, shows the owner's message once and bot messages once", () => {
  const a = [
    item("divider", { at: 0 }),
    item("user", { at: 1000, text: "compare the flows" }),
    item("answer", { text: "I asked B." }),
    item("steps"),
    item("from", { at: 9000, text: "B's flow is X", from: { kind: "bot", id: "b", reply: true } }),
    item("answer", { text: "Both match." }),
  ];
  const b = [
    item("divider", { at: 2000 }),
    item("user", { at: 2000, text: "compare the flows" }),
    item("answer", { text: "On it" }),
    item("from", { at: 5000, text: "What is yours?", from: { kind: "bot", id: "a" } }),
    item("answer", { text: "Mine is X." }),
    item("pending", { text: "and now?" }),
  ];
  const out = mergeConversation([{ bot: { id: "a", name: "A" }, items: a }, { bot: { id: "b", name: "B" }, items: b }], "a");
  const lines = out.map((i) => `${i.bot.name} ${i.t}${i.to ? ` to ${i.to.join(",")}` : ""}${i.text ? `: ${i.text}` : ""}`);
  assert.deepEqual(lines, [
    "A divider",
    "A user to A,B: compare the flows",
    "A answer: I asked B.",
    "A steps",
    "B answer: On it",
    "B from to B: What is yours?",
    "B answer to A: Mine is X.",
    "A answer: Both match.",
    "B pending to B: and now?",
  ]);
});

test("an answer that names its request shows once, in place of its author's answer; others stay as before", () => {
  const a = [
    item("divider", { at: 0 }),
    item("user", { at: 1000, text: "ship today?" }),
    item("answer", { text: "Asked B and C." }),
    // B's answer to r1, linked; C's answer to r2, whose request is not loaded; an older answer without IDs.
    item("from", { key: "sa/e9", at: 9000, text: "Build passed.", from: { kind: "bot", id: "b", reply: true, session: "sb", turn: 5, reply_to: "r1" } }),
    item("from", { at: 9500, text: "Notes ready.", from: { kind: "bot", id: "c", reply: true, session: "sc", turn: 3, reply_to: "r2" } }),
    item("from", { at: 9600, text: "Old.", from: { kind: "bot", id: "b", reply: true, session: "sb" } }),
    item("answer", { text: "We can ship." }),
  ];
  const b = [
    item("divider", { at: 2000 }),
    item("from", { key: "sb/e5", at: 5000, text: "Check the build.", from: { kind: "bot", id: "a", request: "r1" } }),
    item("answer", { key: "sb/e7", text: "Build passed.", turn: "sb/e5" }),
  ];
  const c = [item("divider", { at: 2500 }), item("from", { at: 2500, text: "Check the notes.", from: { kind: "bot", id: "a" } }), item("answer", { key: "sc/e4", text: "Notes ready.", turn: "sc/e3" })];
  const out = mergeConversation([{ bot: { id: "a", name: "A" }, items: a }, { bot: { id: "b", name: "B" }, items: b }, { bot: { id: "c", name: "C" }, items: c }], "a");
  const lines = out.map((i) => `${i.bot.name} ${i.t}${i.to ? ` to ${i.to.join(",")}` : ""}${i.text ? `: ${i.text}` : ""}`);
  assert.deepEqual(lines, [
    "A divider",
    "A user to A: ship today?",
    "A answer: Asked B and C.",
    "C from to C: Check the notes.",
    "C answer to A: Notes ready.",
    "B from to B: Check the build.",
    "A from to A: Build passed.",
    "A answer: We can ship.",
  ]);
  assert.equal(out.find((i) => i.key === "sa/e9").alias, "sb/e7");
});
