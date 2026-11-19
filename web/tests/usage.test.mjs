import assert from "node:assert/strict";
import { test } from "node:test";
const { runAmount, runCost, usd } = await import("../src/lib/usage.ts");
const run = (harness, cost_usd) => ({ started_at: 0, harness, profile: null, input_tokens: 100, cached_tokens: 20, output_tokens: 5, cost_usd });

test("cost counts only positive reported estimates", () => {
  assert.equal(runCost(run("claude", 1.5)), 1.5);
  assert.equal(runCost(run("codex", null)), null);
  assert.equal(runCost(run("claude", 0)), null);
  assert.equal(runCost(run("claude", -1)), null);
  assert.equal(runCost(run("claude", Number.NaN)), null);
  assert.equal(runCost({ ...run("claude", 1), cost_usd: undefined }), null);
});

test("each metric's amount; runs without a cost leave the cost metric only", () => {
  const codex = run("codex", null);
  assert.equal(runAmount(codex, "agents"), 1);
  assert.equal(runAmount(codex, "tokens"), 125);
  assert.equal(runAmount(codex, "cost"), null);
  assert.equal(runAmount(run("claude", 0.42), "cost"), 0.42);
});

test("USD labels", () => {
  assert.equal(usd(0), "$0.00");
  assert.equal(usd(0.001), "<$0.01");
  assert.equal(usd(3.456), "$3.46");
  assert.equal(usd(1234.5), "$1,234.50");
  assert.equal(usd(12_345), "$12.3K");
});
