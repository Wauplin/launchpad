import assert from "node:assert/strict";
import { test } from "node:test";
const { askRecipient, pickRecipient, NO_PICKS } = await import("../src/lib/askRecipient.ts");
const ask = { id: "ask" }, py = { id: "py" }, js = { id: "js" };
const bots = [ask, py, js];
test("defaults: the page's bot, else the Ask bot", () => {
  assert.equal(askRecipient(bots, ask, "w", "/", undefined, NO_PICKS), ask);
  assert.equal(askRecipient(bots, ask, "w", "/bots/py", "py", NO_PICKS), py);
  assert.equal(askRecipient([], null, "w", "/", undefined, NO_PICKS), null);
});
test("a pick elsewhere lasts across pages, yields to a bot page, and stays in its workspace", () => {
  const p = pickRecipient(NO_PICKS, "w", "/", undefined, "js");
  assert.equal(askRecipient(bots, ask, "w", "/agents:", undefined, p), js);
  assert.equal(askRecipient(bots, ask, "w", "/bots/py:", "py", p), py);
  assert.equal(askRecipient(bots, ask, "other", "/", undefined, p), ask);
  assert.equal(askRecipient([ask, py], ask, "w", "/", undefined, p), ask, "a deleted pick falls back");
});
test("a pick on a bot page lasts only on that page and keeps the general pick", () => {
  let p = pickRecipient(NO_PICKS, "w", "/", undefined, "js");
  p = pickRecipient(p, "w", "/bots/py:", "py", "ask");
  assert.equal(askRecipient(bots, ask, "w", "/bots/py:", "py", p), ask);
  assert.equal(askRecipient(bots, ask, "w", "/bots/py:x", "py", p), py);
  assert.equal(askRecipient(bots, ask, "w", "/", undefined, p), js);
  assert.equal(pickRecipient(p, "v", "/", undefined, "py").page, null, "another workspace starts over");
});
