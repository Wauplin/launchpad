import assert from "node:assert/strict";
import { test } from "node:test";
const saved = new Map();
globalThis.localStorage = { getItem: (k) => saved.get(k) ?? null, setItem: (k, v) => saved.set(k, v) };
const listeners = [];
globalThis.window = { addEventListener: (_, fn) => listeners.push(fn) };
const { ordered, useSidebarOrder } = await import("../src/stores/sidebar.ts");
const items = ["a", "b", "c"].map((id) => ({ id }));
test("saved order tolerates removed rows and appends new rows without changing the input", () => {
  assert.deepEqual(ordered(items, ["c", "deleted", "a"]).map((x) => x.id), ["c", "a", "b"]);
  assert.deepEqual(items.map((x) => x.id), ["a", "b", "c"]);
});
test("moves persist per workspace and list, with both directions and invalid drops", () => {
  const move = useSidebarOrder.getState().move;
  move("work:bots", ["a", "b", "c"], "a", "c");
  assert.deepEqual(useSidebarOrder.getState().orders["work:bots"], ["b", "c", "a"]);
  move("work:bots", ["b", "c", "a"], "a", "b");
  assert.deepEqual(useSidebarOrder.getState().orders["work:bots"], ["a", "b", "c"]);
  move("personal:channels", ["x", "y"], "y", "x");
  move("work:bots", ["a", "b", "c"], "missing", "c");
  const stored = JSON.parse(saved.get("launchpad.sidebar.order"));
  assert.deepEqual(stored["work:bots"], ["a", "b", "c"]);
  assert.deepEqual(stored["personal:channels"], ["y", "x"]);
});
test("other browser windows update the order and corrupt storage is ignored", () => {
  saved.set("launchpad.sidebar.order", JSON.stringify({ "work:bots": ["c", "a"], broken: 12 }));
  listeners.forEach((fn) => fn({ storageArea: localStorage, key: "launchpad.sidebar.order" }));
  assert.deepEqual(useSidebarOrder.getState().orders, { "work:bots": ["c", "a"] });
  saved.set("launchpad.sidebar.order", "null");
  listeners.forEach((fn) => fn({ storageArea: localStorage, key: null }));
  assert.deepEqual(useSidebarOrder.getState().orders, {});
});
