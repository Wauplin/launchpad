// Settings → Usage: what one agent run adds to each metric of the activity chart, and how amounts read.
import type { AgentRun } from "@/gen/AgentRun";

export type UsageMetric = "agents" | "tokens" | "cost";

export const tokenCount = (run: AgentRun) => run.input_tokens + run.cached_tokens + run.output_tokens;

/** The run's estimated USD cost when its harness reported one above zero (Claude Code also on a subscription);
 *  null for Codex, unknown pricing or a zero cost, so the run stays out of cost totals and breakdowns. */
export const runCost = (run: AgentRun): number | null =>
  run.cost_usd != null && Number.isFinite(run.cost_usd) && run.cost_usd > 0 ? run.cost_usd : null;

/** What a run adds to a metric; null leaves it out of that metric's chart, legend and totals. */
export function runAmount(run: AgentRun, metric: UsageMetric): number | null {
  if (metric === "agents") return 1;
  if (metric === "tokens") return tokenCount(run);
  return runCost(run);
}

/** A USD amount: cents below $10,000, compact above; "<$0.01" for a cost too small to show. */
export function usd(value: number): string {
  if (value > 0 && value < 0.005) return "<$0.01";
  const big = value >= 10_000;
  return new Intl.NumberFormat("en-US", {
    style: "currency", currency: "USD",
    ...(big ? { notation: "compact", maximumFractionDigits: 1 } : { minimumFractionDigits: 2, maximumFractionDigits: 2 }),
  }).format(value);
}
