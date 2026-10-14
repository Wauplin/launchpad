import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { stripTypeScriptTypes } from "node:module";
import { test } from "node:test";
import { textLimits, MAX_HIGHLIGHT_CHARS } from "../src/features/code/limits.ts";

test("ordinary source retains colors; large source stays readable as plain text", () => {
  assert.deepEqual(textLimits("const x = 1;\n"), { display: null, colors: true });
  const storyboardSized = "x".repeat(450).concat("\n").repeat(304);
  assert.ok(storyboardSized.length > MAX_HIGHLIGHT_CHARS);
  assert.deepEqual(textLimits(storyboardSized), { display: null, colors: false });
  assert.deepEqual(textLimits("x".repeat(2001)), { display: null, colors: false });
  assert.deepEqual(textLimits("\n".repeat(10_000)), { display: null, colors: false });
});

test("extreme lines/counts are refused before rendering or previewing", () => {
  assert.equal(textLimits("x".repeat(20_000)).display, null);
  assert.match(textLimits("x".repeat(20_001)).display, /20,000/);
  assert.equal(textLimits("\n".repeat(99_999)).display, null);
  assert.match(textLimits("\n".repeat(100_000)).display, /100,000/);
});

test("stalled highlighting terminates the worker, settles queued files, and recovers", async (t) => {
  // Load the real client with only its browser logger/worker replaced.
  let source = await readFile(new URL("../src/features/code/highlight.ts", import.meta.url), "utf8");
  source = source.replace('import { logToDaemon } from "@/lib/api";', 'const logToDaemon = () => {};')
    .replace('from "./limits"', `from ${JSON.stringify(new URL("../src/features/code/limits.ts", import.meta.url).href)}`)
    .replace('new URL("./highlight.worker.ts", import.meta.url)', '"worker.js"');
  const instances = [];
  class FakeWorker {
    constructor() { instances.push(this); }
    postMessage(message) { this.message = message; }
    terminate() { this.terminated = true; }
    answer(tokens) { this.onmessage({ data: { id: this.message.id, tokens } }); }
  }
  const original = globalThis.Worker;
  globalThis.Worker = FakeWorker;
  t.after(() => { globalThis.Worker = original; });
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { highlight } = await import(`data:text/javascript;base64,${Buffer.from(stripTypeScriptTypes(source)).toString("base64")}`);
  assert.equal(await highlight("large", "x\n".repeat(MAX_HIGHLIGHT_CHARS), "html", "light"), null);
  assert.equal(instances.length, 0);
  const first = highlight("stuck", "<div>", "html", "light");
  const queued = highlight("queued", "a", "javascript", "light");
  t.mock.timers.tick(5000);
  assert.equal(await first, null);
  assert.equal(await queued, null);
  assert.equal(instances[0].terminated, true);
  const next = highlight("next", "ok", "javascript", "light");
  assert.equal(instances.length, 2);
  const tokens = [[["ok", "#fff", 0]]];
  instances[1].answer(tokens);
  assert.deepEqual(await next, tokens);
  assert.deepEqual(await highlight("next", "ok", "javascript", "light"), tokens);
  const failed = highlight("failed", "bad", "javascript", "dark");
  instances[1].onerror({ message: "load failure" });
  assert.equal(await failed, null);
  assert.equal(instances[1].terminated, true);
});
