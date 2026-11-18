// The sketch frame's data bridge checks and its document (web/src/lib/folioSketch.ts).
import { test } from "node:test";
import assert from "node:assert/strict";
import { refuse, relativeSources, sketchDoc, sketchPrefix, SKETCH_CSP } from "../src/lib/folioSketch.ts";

test("prefixes match Home's (folio_check::sketch_prefix)", () => {
  assert.equal(sketchPrefix(null, "sketches/inbox.html", "S-1"), "inbox:");
  assert.equal(sketchPrefix(null, null, "S-1"), "S-1:");
  assert.equal(sketchPrefix("poll:", "sketches/a.html", null), "poll:");
  assert.equal(sketchPrefix(null, null, null), "sketch:");
});

test("the bridge refuses keys outside the prefix, writes on read-only views and big values", () => {
  const call = (op, key, value) => ({ lp: "t", id: 1, op, key, value });
  assert.equal(refuse(call("get", "inbox:read"), "inbox:", false), null);
  assert.match(refuse(call("get", "ranking"), "inbox:", false), /outside/);
  assert.equal(refuse(call("list", ""), "inbox:", false), null);
  assert.match(refuse(call("set", "inbox:a", 1), "inbox:", true), /read-only/);
  assert.match(refuse(call("set", "inbox:a", "x".repeat(70_000)), "inbox:", false), /64 KB/);
  assert.match(refuse(call("set", "inbox:a b", 1), "inbox:", false), /letters/);
  assert.match(refuse(call("eval", "inbox:a"), "inbox:", false), /unknown/);
});

test("the document starts with the CSP, before the sketch", () => {
  const doc = sketchDoc({ html: "<p>hi</p><script>fetch('https://x')</script>", kit: null, theme: { "--color-ink": "red</style>" }, prefix: "a:", token: "t" });
  assert.ok(doc.indexOf(SKETCH_CSP) > 0 && doc.indexOf(SKETCH_CSP) < doc.indexOf("<p>hi</p>"));
  assert.ok(!doc.includes("red</style>"), "theme values cannot close the style element");
  assert.ok(SKETCH_CSP.includes("connect-src 'none'") && SKETCH_CSP.includes("default-src 'none'"));
  assert.deepStrictEqual(relativeSources('<img src="../assets/a.png"><img src="https://x/b.png"><img src=\'c.svg\'>'), ["../assets/a.png", "c.svg"]);
});
