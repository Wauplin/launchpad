import assert from "node:assert/strict";
import { test } from "node:test";
import { clampColumn, columnMax, COLUMNS, parseWidths } from "../src/lib/columns.ts";

const sidebar = COLUMNS.sidebar;

test("a column without a stored width has its default width", () => {
  assert.equal(clampColumn(sidebar, undefined, 1600), sidebar.initial);
  assert.equal(clampColumn(sidebar, Number.NaN, 1600), sidebar.initial);
  assert.equal(clampColumn(sidebar, -5, 1600), sidebar.initial);
});

test("a width stays within the column's min and max", () => {
  assert.equal(clampColumn(sidebar, 20, 1600), sidebar.min);
  assert.equal(clampColumn(sidebar, 5000, 4000), sidebar.max);
  assert.equal(clampColumn(sidebar, 250.4, 1600), 250);
});

test("a narrow window lowers the max, never under the min", () => {
  assert.equal(columnMax(sidebar, 800), 240);
  assert.equal(clampColumn(sidebar, 300, 800), 240);
  assert.equal(columnMax(sidebar, 100), sidebar.min);
  assert.equal(columnMax(sidebar, 0), sidebar.max);
});

test("only known columns with usable widths are read back", () => {
  assert.deepEqual(parseWidths('{"sidebar": 240, "agents-list": "wide", "other": 300, "files-tree": 0}'), { sidebar: 240 });
  assert.deepEqual(parseWidths("not json"), {});
  assert.deepEqual(parseWidths("[1]"), {});
  assert.deepEqual(parseWidths(null), {});
});
