// The UI's folio splitter against the fixtures shared with Home's (crates/automation/src/folio_split.rs): each
// web/tests/folio-split/<name>.md gives exactly the outline of <name>.json on both sides.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { attrs, outline, slug } from "../src/lib/folioSplit.ts";

const dir = new URL("./folio-split/", import.meta.url);

test("shared fixtures give the same outline as Home's splitter", () => {
  const pages = readdirSync(dir).filter((f) => f.endsWith(".md"));
  assert.ok(pages.length >= 3);
  for (const f of pages) {
    const text = readFileSync(new URL(f, dir), "utf8");
    const want = JSON.parse(readFileSync(new URL(f.replace(/\.md$/, ".json"), dir), "utf8"));
    assert.deepStrictEqual(outline(text), want, f);
  }
});

test("slugs and attributes", () => {
  assert.equal(slug("What [blocks](x.md) us?"), "what-blocks-us");
  assert.equal(slug("¿?"), "section");
  assert.equal(slug("Déjà vu"), "déjà-vu");
  const a = attrs('{#d1 .wide tone=verdict title="Two words" open}');
  assert.deepStrictEqual(a, { id: "d1", classes: ["wide"], pairs: { tone: "verdict", title: "Two words" }, flags: ["open"] });
});
