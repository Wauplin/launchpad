import assert from "node:assert/strict";
import { test } from "node:test";
import { placePanel } from "../src/components/popoverPlace.ts";

const view = { x: 0, y: 0, w: 1200, h: 500 };
const panel = { width: 360, scrollHeight: 300 };

test("a popover opens under its anchor when it fits", () => {
  const p = placePanel({ left: 20, right: 100, top: 40, bottom: 70 }, view, panel, "left", 1);
  assert.equal(p.top, 74);
  assert.equal(p.left, 20);
});

test("a popover in a bottom action bar opens upward, kept on screen", () => {
  const p = placePanel({ left: 20, right: 100, top: 440, bottom: 470 }, view, panel, "left", 1);
  assert.equal(p.top, 440 - 4 - 300);
  assert.equal(p.maxHeight, 428);
});

test("a popover taller than the room scrolls inside, never above the view", () => {
  const p = placePanel({ left: 20, right: 100, top: 440, bottom: 470 }, view, { width: 360, scrollHeight: 900 }, "left", 1);
  assert.equal(p.maxHeight, 428);
  assert.equal(p.top, 8);
});

test("right-aligned popovers end at their anchor; both stay inside the view", () => {
  assert.equal(placePanel({ left: 1100, right: 1130, top: 40, bottom: 70 }, view, panel, "right", 1).left, 1130 - 360);
  assert.equal(placePanel({ left: 1100, right: 1130, top: 40, bottom: 70 }, view, panel, "left", 1).left, 1200 - 360 - 8);
  assert.equal(placePanel({ left: 2, right: 30, top: 40, bottom: 70 }, view, panel, "right", 1).left, 8);
});

test("a zoomed root converts viewport pixels to CSS pixels; an anchor out of view closes", () => {
  assert.equal(placePanel({ left: 24, right: 120, top: 48, bottom: 84 }, view, panel, "left", 1.2).top, 88 / 1.2);
  assert.equal(placePanel({ left: 20, right: 100, top: 600, bottom: 630 }, view, panel, "left", 1), null);
});
