// Agents part of the demo world: sessions with their stored events, scheduled launches, crons, the Ask thread and its
// proposals, task worktrees and session Git views. Sessions belong to workspaces through their folder. Several
// sessions are prefilled from the prerecorded scenarios (`scenarios/`), so they continue live when answered or when
// the demo clock passes a quota reset.

import type { CronAgent } from "@/gen/CronAgent";
import type { EventBody } from "@/gen/EventBody";
import type { FileAttachment } from "@/gen/FileAttachment";
import type { LaunchDelay } from "@/gen/LaunchDelay";
import type { LaunchSpec } from "@/gen/LaunchSpec";
import type { Proposal } from "@/gen/Proposal";
import type { ScheduledLaunch } from "@/gen/ScheduledLaunch";
import type { Session } from "@/gen/Session";
import type { SessionEvent } from "@/gen/SessionEvent";
import type { TaskWorktree } from "@/gen/TaskWorktree";
import { clock } from "../daemon";
import { gitPatch, prefill, type AgentsState, type Options } from "../player";
import { QUOTA_RESET_MINUTES } from "../scenarios/quota";
import "../scenarios";
import { defaultSpec } from "../store";
import { P, ROOT, WS } from "./ids";

export { QUOTA_RESET_MINUTES };

/** Neutral model IDs per harness (no real model names that would go stale). */
export const MODELS: Record<string, string> = { claude: "demo-large", codex: "demo-codex", opencode: "router/demo-coder" };
export const PROFILE: Record<string, string> = { claude: "claude-max", codex: "codex-plus", opencode: "opencode-router" };

/** Folder name of each demo folder ID. */
export const FOLDER: Record<string, string> = { [P.storefront]: "storefront", [P.billing]: "billing-api", [P.docs]: "docs", [P.dotfiles]: "dotfiles", [P.scratch]: "tmp-1028-csv-cleanup" };
const WORKSPACE_OF: Record<string, string | null> = { [P.storefront]: WS.acme, [P.billing]: WS.acme, [P.docs]: WS.acme, [P.dotfiles]: WS.personal, [P.scratch]: null };
export const workspaceOf = (projectId: string | null) => (projectId ? WORKSPACE_OF[projectId] ?? null : null);

/** The text `compose.rs` appends to every initial launch prompt (folded under + Launchpad instructions). */
export const SUMMARY_INSTRUCTION = "Launchpad final message: when you finish the initial task, make your last message a concise, self-contained summary for the user to read in the agent run view. Lead with the outcome and cover all substantive results, checks actually performed and their results, and any remaining work, blockers or limitations. Include relevant links or file paths when useful; omit missing details and routine narration. This applies only to the initial task, not subsequent turns. If the user steers you or requests a specific answer or format, follow that request instead; a work summary is not required.";

const hash40 = (seed: string) => {
  let h = 0x811c9dc5;
  let out = "";
  while (out.length < 40) { for (let i = 0; i < seed.length; i++) h = Math.imul(h ^ seed.charCodeAt(i), 16777619); out += (h >>> 0).toString(16).padStart(8, "0"); }
  return out.slice(0, 40);
};

/** A task worktree record for a session (`.worktrees/<folder id>/<slug>`, branch `lp/<slug>-<session8>`). */
export function taskWorktree(sessionId: string, projectId: string, slug: string, createdAt: number): TaskWorktree {
  const repo = `${ROOT}/${FOLDER[projectId] ?? projectId}`;
  return {
    id: `wt-${sessionId}`, sessions: [sessionId], project_id: projectId, repo, common_dir: `${repo}/.git`, root: `${ROOT}/.worktrees/${projectId}/${slug}`,
    subdir: "", branch: `lp/${slug}-${sessionId.replace(/^demo-(s|run)-/, "").replace(/[^a-z0-9]/gi, "").slice(0, 8)}`, base_branch: "main", base_commit: hash40(`${projectId}:main`),
    primary: true, state: "active", merge: null, merged: null, predraft_id: null, handoff_from: null, cleanup_note: null, created_at: createdAt,
  };
}

export const usage = (k: number, cost: number | null = null) => ({ context_tokens: 18_000 + k * 900, context_window: 200_000, input_tokens: k * 9_000, cached_tokens: k * 6_500, output_tokens: k * 420, cost_usd: cost, cost_unavailable: false });

type Build = {
  id: string;
  title: string;
  project: string | null;
  harness: string;
  minutesAgo: number;
  status?: Session["status"];
  events?: EventBody[];
  scenario?: { id: string; options: Options };
  worktree?: string;
  seen?: boolean;
  spec?: Partial<LaunchSpec>;
  cwd?: string;
  patch?: Partial<Session>;
};

/** A demo session whose stored events are `events`, spaced out over the last `minutesAgo` minutes. */
export function demoSession(b: Build, log: Record<string, SessionEvent[]>, events: EventBody[]): Session {
  const started = clock.ago(b.minutesAgo);
  const span = b.minutesAgo * 60_000 * 0.85;
  log[b.id] = events.map((body, i) => ({ session_id: b.id, seq: i + 1, ts: started + Math.round(((i + 1) / (events.length + 1)) * span), body }));
  const last = [...events].reverse().find((e) => e.kind === "assistant_message");
  const status = b.status ?? "waiting";
  const lastTs = log[b.id].at(-1)?.ts ?? started;
  const typed = b.spec?.text ?? b.title;
  return {
    id: b.id, harness: b.harness, harness_session_id: `native-${b.id}`, project_id: b.project, cwd: b.cwd ?? `${ROOT}/${b.project ? FOLDER[b.project] : "scratch"}`, title: b.title, status,
    archived_done: false, status_since: lastTs, live: ["starting", "working", "background", "waiting", "quota_paused"].includes(status),
    open_question: false, model: MODELS[b.harness] ?? null, effort: b.harness === "codex" ? "medium" : null, prompt: `${typed}\n\n${SUMMARY_INSTRUCTION}`,
    usage: usage(Math.max(2, Math.round(events.length / 2))), started_at: started, ended_at: null, last_seq: events.length,
    launch_spec: defaultSpec({ harness: b.harness, profile: PROFILE[b.harness] ?? null, target: b.project ? { kind: "project", project_id: b.project } : { kind: "temp" }, text: typed, workspace_id: workspaceOf(b.project), ...b.spec }),
    capacity_retry_count: 0, capacity_retry_at: null,
    quota_pause: null, seen_at: b.seen === false ? 0 : lastTs + 30_000,
    last_message: last && last.kind === "assistant_message" ? last.text.split("\n").find((l) => l.trim())?.replace(/[*_`#]/g, "").slice(0, 400) ?? null : null,
    ...b.patch,
  };
}

// ---------------------------------------------------------------- event helpers for hand-written sessions

let item = 0;
const id = (k: string) => `${k}${++item}`;
const user = (text: string, instructions = true): EventBody => ({ kind: "user_message", text: instructions ? `${text}\n\n${SUMMARY_INSTRUCTION}` : text, client_id: id("c"), queued: false, images: [] });
const think = (text: string): EventBody => ({ kind: "reasoning", item_id: id("r"), text });
const say = (text: string): EventBody => ({ kind: "assistant_message", item_id: id("m"), text });
function tool(name: string, cmd: string, output: string, exit = 0): EventBody[] {
  const i = id("t");
  return [{ kind: "tool_call", item_id: i, name, summary: cmd, input: { command: cmd } }, { kind: "tool_result", item_id: i, output, is_error: exit !== 0, exit_code: exit, duration_ms: 1800 }];
}
function change(name: string, path: string, diff: string, kind = "update"): EventBody[] {
  const i = id("e");
  return [{ kind: "tool_call", item_id: i, name, summary: path, input: { file_path: path } }, { kind: "file_change", item_id: i, path, change: kind, diff }, { kind: "tool_result", item_id: i, output: `Updated ${path}`, is_error: false, exit_code: null, duration_ms: 300 }];
}
const end = (minutes = 3, interrupted = false, error: string | null = null): EventBody[] => [{ kind: "turn_end", duration_ms: minutes * 60_000, interrupted, error }];
const status = (s: Session["status"]): EventBody => ({ kind: "status", status: s });
const sub = (agent: string, body: EventBody): EventBody => ({ kind: "subagent_event", agent_id: agent, body });

// ---------------------------------------------------------------- the world

export function agentsWorld() {
  item = 0;
  const state: AgentsState = { sessions: [], events: {}, sessionGit: {}, agentWorktrees: [], proposals: [], taskOutputs: {}, dormant: {} };
  const add = (b: Build) => {
    let events = b.events ?? [];
    // The typed text is the start of the first message (the trace folds what compose.rs appended).
    const first = events[0];
    if (!b.spec?.text && first?.kind === "user_message") b = { ...b, spec: { ...b.spec, text: first.text.split(`\n\n${SUMMARY_INSTRUCTION}`)[0] } };
    if (b.worktree && b.project) {
      const wt = taskWorktree(b.id, b.project, b.worktree, clock.ago(b.minutesAgo));
      state.agentWorktrees.push(wt);
      b = { ...b, cwd: wt.root, spec: { ...b.spec, worktree: { base: null, include_changes: false, merge_when_done: !!b.scenario?.options.merge } } };
    }
    if (b.scenario) b = { ...b, spec: { permission_mode: b.scenario.options.permission, ...b.spec } };
    const s = demoSession(b, state.events, events);
    state.sessions.push(s);
    if (b.scenario) {
      const played = prefill(state, b.id, b.scenario.id, b.scenario.options);
      events = [user(b.spec?.text ?? b.title), ...played.events];
      const fresh = demoSession({ ...b, status: played.patch.status ?? "waiting" }, state.events, events);
      Object.assign(s, fresh, { open_question: !!played.patch.open_question, quota_pause: played.patch.quota_pause ?? null });
      // Quota resets are relative to now, not to the spaced-out event times.
      if (s.status === "quota_paused") s.seen_at = s.status_since;
    }
    return s;
  };

  add({
    id: "demo-s-synced", title: "Review the billing retry trace from my laptop", project: P.billing,
    harness: "codex", status: "waiting", minutesAgo: 180,
    spec: { profile: null, harness: null, workspace_id: null },
    patch: { imported_from: "hf://buckets/demo/launchpad-traces/billing-api/20261030_1200_demo", live: false, harness_session_id: null },
    events: [user("Review billing retry behavior"), { kind: "assistant_message", item_id: "synced-result", text: "The retry handler preserves the request ID. A local follow-up can add coverage for delayed responses." }],
  });

  // Needs you: one question (Ask for approval mode, an approval card follows), two unseen Done, one Failed.
  add({ id: "demo-s-invoices", title: "Rate-limit /invoices", project: P.billing, harness: "codex", minutesAgo: 6, worktree: "rate-limit-invoices",
    spec: { text: "Rate-limit GET /invoices to 10 requests per minute per API key. The tests in tests/rate_limit.rs describe the behavior." },
    scenario: { id: "rate-limit", options: { permission: "approvals", merge: false } } });
  add({ id: "demo-s-checkout", title: "Fix the flaky checkout test", project: P.storefront, harness: "claude", minutesAgo: 24, worktree: "fix-flaky-checkout-test", seen: false,
    spec: { text: "Fix the flaky checkout test: it fails about one CI run in ten." }, scenario: { id: "checkout", options: { permission: "yolo", merge: false } } });
  add({ id: "demo-s-docs", title: "Document the refund flow", project: P.docs, harness: "opencode", minutesAgo: 95, seen: false, spec: { budget_usd: 2 }, cwd: `${ROOT}/.worktrees/${P.docs}/document-refund-flow`,
    patch: { usage: usage(6, 0.41) },
    events: [
      user("Document the refund flow for support agents: states, who can refund, and two examples."),
      think("The refund states live in billing-api/src/refunds/state.rs. I will describe them in plain words, with a diagram."),
      ...tool("read", "../billing-api/src/refunds/state.rs", "pub enum RefundState { Requested, Approved, Sent, Failed }"),
      ...change("write", "guides/refunds.md", "+# Refunds\n+\n+A refund moves through four states:\n+\n+```mermaid\n+stateDiagram-v2\n+  Requested --> Approved\n+  Approved --> Sent\n+  Approved --> Failed\n+  Failed --> Approved: retry\n+```\n+\n+Only **billing admins** can approve a refund above 500 credits.", "add"),
      say("**Added `guides/refunds.md`** with the four refund states, a state diagram, the approval rule for large refunds and two worked examples (a partial refund and a failed card).\n\n- Checked: `pnpm docs:check`, no broken links.\n- Not done: screenshots of the admin page; they need a staging login."),
      ...end(4), status("waiting"),
    ] });
  add({ id: "demo-s-node24", title: "Upgrade CI to Node 24", project: P.storefront, harness: "codex", minutesAgo: 70, seen: false, status: "error",
    events: [
      user("Upgrade CI to Node 24 and make the workflow pass."),
      think("Updating the setup-node version in the CI workflow, then installing with the same flags as CI."),
      ...change("apply_patch", ".github/workflows/ci.yml", "@@ -14,7 +14,7 @@ jobs:\n       - uses: actions/setup-node@v4\n         with:\n-          node-version: 20\n+          node-version: 24\n           cache: pnpm"),
      ...tool("Shell", "pnpm install --frozen-lockfile", " ERR_PNPM_LOCKFILE_BREAKING_CHANGE  Lockfile /home/demo/projects/storefront/pnpm-lock.yaml not compatible with current pnpm\n\nRun with --force to recreate the lockfile.", 1),
      { kind: "error", message: "The turn failed: the model stream ended unexpectedly (connection reset). Resume to retry." },
      ...end(2, false, "stream ended unexpectedly"), status("error"),
    ] });

  // Running: two looping replays and one with a background command.
  add({ id: "demo-s-darkmode", title: "Add a dark-mode toggle to the header", project: P.storefront, harness: "claude", minutesAgo: 3, status: "working", worktree: "dark-mode-toggle",
    spec: { text: "Add a dark-mode toggle to the site header. Remember the choice." }, scenario: { id: "loop-dark-mode", options: { permission: "yolo", merge: false, loop: true } } });
  add({ id: "demo-s-pdf", title: "Speed up invoice PDF rendering", project: P.billing, harness: "opencode", minutesAgo: 2, status: "working", patch: { usage: usage(5, 0.27) },
    spec: { text: "Invoice PDFs take 400 ms for ten pages. Find where the time goes and make it faster." }, scenario: { id: "loop-pdf", options: { permission: "yolo", merge: false, loop: true } } });
  add({ id: "demo-s-loadtest", title: "Run the load test on the invoices API", project: P.billing, harness: "claude", minutesAgo: 12, status: "background",
    events: [
      user("Run the k6 load test against the local invoices API and tell me the p95 latency."),
      think("The load test takes about 15 minutes. I will start it in the background and check its output."),
      { kind: "tool_call", item_id: "bg-call", name: "Bash", summary: "k6 run loadtest/invoices.js", input: { command: "k6 run loadtest/invoices.js", run_in_background: true } },
      { kind: "tool_result", item_id: "bg-call", output: "Command running in background with ID: bash_1. Output is being written to: /tmp/claude/bash_1.out", is_error: false, exit_code: null, duration_ms: 120 },
      { kind: "background_task", task_id: "bash_1", description: "k6 run loadtest/invoices.js", status: "running", output_file: "/tmp/claude/bash_1.out" },
      say("The load test runs in the background (about 15 minutes). I will report the p95 latency when it finishes."),
      ...end(1), status("background"),
    ] });
  state.taskOutputs["demo-s-loadtest"] = { bash_1: "\n     execution: local\n        script: loadtest/invoices.js\n\n  scenarios: (100.00%) 1 scenario, 50 max VUs, 15m30s max duration\n\nrunning (04m12.0s), 50/50 VUs, 12408 complete and 0 interrupted iterations\ndefault   [====>---------------------------------] 50 VUs  04m12.0s/15m0s\n\n     http_req_duration: avg=41.8ms p(90)=88.1ms p(95)=112.4ms" };

  // Quota paused: continues when the demo clock passes the 5-hour reset.
  add({ id: "demo-s-quota", title: "Split the invoice service into modules", project: P.billing, harness: "claude", minutesAgo: 9, worktree: "split-invoice-service",
    spec: { text: "Split src/invoices/service.rs into modules (totals, numbering, pdf) without changing behavior." }, scenario: { id: "quota", options: { permission: "yolo", merge: false } } });

  // Done and seen.
  add({ id: "demo-s-gitalias", title: "Add a git alias for the log graph", project: P.dotfiles, harness: "claude", minutesAgo: 60 * 26,
    events: [
      user("Add a git alias `lg` that shows the log as a graph with relative dates."),
      ...tool("Read", "~/dotfiles/git/config", "[alias]\n  st = status -sb\n  co = checkout"),
      ...change("Edit", "git/config", "-  co = checkout\n+  co = checkout\n+  lg = log --graph --pretty=format:'%C(yellow)%h%Creset %s %C(green)(%cr)%Creset %C(blue)<%an>%Creset' --abbrev-commit"),
      say("Added **`git lg`** to `git/config`: a one-line graph with the short hash, subject, relative date and author. Try it with `git lg -20`."),
      ...end(1), status("waiting"),
    ] });
  add({ id: "demo-s-csv", title: "Clean up the customer CSV export", project: P.scratch, harness: "claude", minutesAgo: 60 * 5,
    events: [
      user("customers.csv has duplicate rows and mixed date formats. Write a script that cleans it."),
      ...tool("Bash", "head -5 customers.csv", "id,name,email,signup\n17,Ada Park,ada@example.com,2024-03-02\n17,Ada Park,ada@example.com,03/02/2024\n18,Lin Ortiz,lin@example.com,2024-03-05"),
      ...change("Write", "clean.py", "+import csv, datetime\n+\n+def parse(d):\n+    for fmt in (\"%Y-%m-%d\", \"%m/%d/%Y\"):\n+        try: return datetime.datetime.strptime(d, fmt).date()\n+        except ValueError: pass\n+    raise ValueError(d)", "add"),
      ...tool("Bash", "python3 clean.py customers.csv > customers.clean.csv && wc -l customers*.csv", "  4211 customers.csv\n  3980 customers.clean.csv"),
      say("**`customers.clean.csv` is ready**: 231 duplicate rows removed, every date in ISO format (`YYYY-MM-DD`). The script is `clean.py` in this scratch folder."),
      ...end(2), status("waiting"),
    ] });
  add({ id: "demo-s-audit", title: "Nightly dependency audit", project: P.billing, harness: "codex", minutesAgo: 60 * 9,
    spec: { text: "Run cargo audit and pnpm audit. Report new advisories only, with the fix for each." },
    events: [
      user("Run cargo audit and pnpm audit. Report new advisories only, with the fix for each."),
      ...tool("Shell", "cargo audit", "Crate:     time\nVersion:   0.3.28\nTitle:     Potential segfault in localtime_r\nSolution:  Upgrade to >=0.3.36\n\nerror: 1 vulnerability found!", 1),
      say("**One new advisory.**\n\n| Crate | Version | Fix |\n| --- | --- | --- |\n| `time` | 0.3.28 | upgrade to 0.3.36 (`cargo update -p time`) |\n\nNo new advisories in the JavaScript dependencies."),
      ...end(2), status("waiting"),
    ] });

  // Stopped, Interrupted, In terminal.
  add({ id: "demo-s-rename", title: "Rename the OrderSummary props", project: P.storefront, harness: "codex", minutesAgo: 140, status: "stopped",
    events: [
      user("Rename the OrderSummary props to match the API names (subtotal, taxTotal, grandTotal)."),
      ...tool("Shell", "rg -l \"OrderSummary\" src", "src/components/OrderSummary.tsx\nsrc/pages/cart.tsx\nsrc/pages/checkout.tsx"),
      ...change("apply_patch", "src/components/OrderSummary.tsx", "@@ -3,5 +3,5 @@\n type Props = {\n-  sub: number;\n-  tax: number;\n-  total: number;\n+  subtotal: number;\n+  taxTotal: number;\n+  grandTotal: number;\n };"),
      ...end(1, true), status("stopped"),
    ] });
  add({ id: "demo-s-apidocs", title: "Regenerate the API reference", project: P.docs, harness: "claude", minutesAgo: 60 * 3,
    status: "interrupted",
    events: [
      user("Regenerate the API reference from the billing-api OpenAPI file."),
      ...tool("Bash", "pnpm openapi:generate ../billing-api/openapi.yaml", "Generating 48 endpoints…"),
      { kind: "notice", text: "Launchpad restarted while this turn was running. The turn was interrupted; Resume continues it." },
      ...end(1, true), status("interrupted"),
    ] });
  add({ id: "demo-s-cart", title: "Debug the cart total rounding", project: P.storefront, harness: "codex", minutesAgo: 40, status: "in_terminal",
    events: [
      user("The cart total is off by one cent for some discounts. Find out why."),
      ...tool("Shell", "pnpm vitest run tests/cart.test.ts -t rounding", " FAIL tests/cart.test.ts > rounding > applies 15% to 19.99\n expected 1699 to be 1700", 1),
      say("The discount is rounded per line, then summed. I want to try two rounding orders interactively."),
      ...end(1),
      { kind: "notice", text: "Continued in the Codex TUI in a terminal. Re-attach to continue here." },
      status("in_terminal"),
    ] });

  // Archived: one with subagents, one on the SSH machine gpu-box.
  const a11y = add({ id: "demo-s-a11y", title: "Audit accessibility of the checkout pages", project: P.storefront, harness: "claude", minutesAgo: 60 * 30, status: "closed",
    events: [
      user("Audit the accessibility of the cart and checkout pages. Use one subagent per page and merge the findings."),
      think("Two independent pages: I will delegate each one to a subagent, then merge the findings into one list."),
      { kind: "tool_call", item_id: "task-cart", name: "Task", summary: "Audit the cart page", input: { description: "Audit the cart page", prompt: "Audit src/pages/cart.tsx for WCAG 2.2 AA issues." } },
      { kind: "subagent", agent_id: "task-cart", parent_id: null, name: "Audit the cart page", status: "running", model: "demo-medium" },
      { kind: "tool_call", item_id: "task-checkout", name: "Task", summary: "Audit the checkout page", input: { description: "Audit the checkout page", prompt: "Audit src/pages/checkout.tsx for WCAG 2.2 AA issues." } },
      { kind: "subagent", agent_id: "task-checkout", parent_id: null, name: "Audit the checkout page", status: "running", model: "demo-medium" },
      sub("task-cart", { kind: "reasoning", item_id: "c-r1", text: "Checking labels, focus order and contrast on the cart page." }),
      sub("task-cart", { kind: "tool_call", item_id: "c-t1", name: "Grep", summary: "rg -n \"<input\" src/pages/cart.tsx", input: { pattern: "<input" } }),
      sub("task-cart", { kind: "tool_result", item_id: "c-t1", output: "42:      <input type=\"number\" value={qty} onChange={setQty} />", is_error: false, exit_code: null, duration_ms: 200 }),
      sub("task-checkout", { kind: "reasoning", item_id: "k-r1", text: "Checking the payment form fields and error messages." }),
      sub("task-checkout", { kind: "tool_call", item_id: "k-t1", name: "Read", summary: "src/pages/checkout.tsx", input: { file_path: "src/pages/checkout.tsx" } }),
      sub("task-checkout", { kind: "tool_result", item_id: "k-t1", output: "…<span className=\"error\">{error}</span>…", is_error: false, exit_code: null, duration_ms: 150 }),
      sub("task-cart", { kind: "assistant_message", item_id: "c-m1", text: "Cart: the quantity input has no label (line 42), and the remove button is an icon without an accessible name." }),
      { kind: "subagent", agent_id: "task-cart", parent_id: null, name: "Audit the cart page", status: "completed", model: "demo-medium" },
      sub("task-checkout", { kind: "assistant_message", item_id: "k-m1", text: "Checkout: payment errors are not announced (no role=alert), and the card fields lack autocomplete attributes." }),
      { kind: "subagent", agent_id: "task-checkout", parent_id: null, name: "Audit the checkout page", status: "completed", model: "demo-medium" },
      { kind: "tool_result", item_id: "task-cart", output: "Cart: 2 issues", is_error: false, exit_code: null, duration_ms: 64_000 },
      { kind: "tool_result", item_id: "task-checkout", output: "Checkout: 2 issues", is_error: false, exit_code: null, duration_ms: 71_000 },
      say("**4 accessibility issues** on the two pages:\n\n1. Cart: the quantity input has no label.\n2. Cart: the remove button has no accessible name.\n3. Checkout: payment errors are not announced (`role=\"alert\"` missing).\n4. Checkout: card fields lack `autocomplete` attributes.\n\nAll four are small fixes; none needs a design change."),
      ...end(3), status("waiting"), status("closed"),
    ] });
  a11y.archived_done = true;
  a11y.live = false;
  a11y.ended_at = a11y.status_since;
  const gpu = add({ id: "demo-s-gpubench", title: "Benchmark the reconciliation job on gpu-box", project: P.billing, harness: "codex", minutesAgo: 60 * 50, status: "closed",
    cwd: "gpu-box:/home/demo/projects/billing-api",
    events: [
      user("On gpu-box, benchmark the nightly reconciliation job with the full March data set."),
      ...tool("Shell", "cargo run --release --bin reconcile -- --month 2026-03 --dry-run", "Loaded 4,812,331 ledger rows in 6.2s\nMatched 4,809,977 (99.95%)\nUnmatched 2,354 → reports/unmatched-2026-03.csv\nTotal: 41.8s"),
      say("**The reconciliation of March takes 41.8 s on gpu-box** (6.2 s loading, 35.6 s matching). 2,354 rows are unmatched; they are listed in `reports/unmatched-2026-03.csv` on gpu-box."),
      ...end(2), status("waiting"), status("closed"),
    ] });
  gpu.archived_done = true;
  gpu.live = false;
  gpu.ended_at = gpu.status_since;

  // Ask Launchpad: the Acme thread, with a short prerecorded conversation.
  const assistantCwd = `/home/demo/.local/share/launchpad/assistant/${WS.acme}`;
  add({ id: "demo-s-ask-acme", title: "Ask Launchpad · Acme", project: null, harness: "claude", minutesAgo: 35, cwd: assistantCwd,
    spec: { target: { kind: "assistant", workspace_id: WS.acme }, workspace_id: WS.acme, text: "What is running in Acme right now?" },
    events: [
      user("What is running in Acme right now?", false),
      ...tool("Bash", "launchpad sessions list --workspace Acme", "[{\"title\":\"Add a dark-mode toggle to the header\",\"status\":\"working\"},{\"title\":\"Speed up invoice PDF rendering\",\"status\":\"working\"},{\"title\":\"Run the load test on the invoices API\",\"status\":\"background\"}, …]"),
      say("Three agents run in **Acme**:\n\n- **Add a dark-mode toggle to the header** (storefront): testing the toggle.\n- **Speed up invoice PDF rendering** (billing-api): benchmarking font caching.\n- **Run the load test on the invoices API** (billing-api): k6 runs in the background, about 10 minutes left."),
      ...end(1),
      user("Anything I should start or automate?", false),
      ...tool("Bash", "launchpad inbox list --workspace Acme", "[{\"repo\":\"acme-labs/billing-api\",\"number\":77,\"reason\":\"mention\",\"title\":\"Invoices API returns 500 on empty cart\"}, …]"),
      ...tool("Bash", "launchpad propose launch --title \"Fix the empty-cart 500 in the invoices API\" --spec-file /tmp/fix.json", "Proposal created (waiting for the owner)"),
      ...tool("Bash", "launchpad propose cron --title \"Weekly docs link check\" --spec-file /tmp/links.json", "Proposal created (waiting for the owner)"),
      say("I proposed two things; nothing starts until you press its button:\n\n1. **Fix the empty-cart 500 in the invoices API**: issue #77 mentions you, and the fix looks small.\n2. **Weekly docs link check**: a repeating agent every Monday at 09:00 that reports broken links in docs.\n\nBoth cards are in **Proposals**."),
      ...end(1), status("waiting"),
    ] });
  const proposals: Proposal[] = [
    { id: "demo-prop-empty-cart", workspace_id: WS.acme, session_id: "demo-s-ask-acme", title: "Fix the empty-cart 500 in the invoices API",
      note: "Issue #77 mentions you: `GET /invoices` returns 500 when the cart is empty. The fix is likely a missing empty check.",
      body: { kind: "launch", spec: defaultSpec({ harness: "codex", profile: "codex-plus", target: { kind: "project", project_id: P.billing }, worktree: { base: null, include_changes: false, merge_when_done: false }, text: "Fix acme-labs/billing-api#77: GET /invoices returns 500 when the cart is empty. Add a regression test.", workspace_id: WS.acme }) },
      state: "proposed", result_id: "demo-s-proposed-cart", replaced_by: null, error: null, created_at: clock.ago(31), updated_at: clock.ago(31) },
    { id: "demo-prop-links", workspace_id: WS.acme, session_id: "demo-s-ask-acme", title: "Weekly docs link check",
      note: "Broken links in docs were reported twice this month. A weekly check catches them before customers do.",
      body: { kind: "cron", spec: defaultSpec({ harness: "opencode", profile: "opencode-router", target: { kind: "project", project_id: P.docs }, text: "Run pnpm docs:check and report broken links with the page and the fix. Do not change files.", workspace_id: WS.acme }), schedule: "0 9 * * 1", conditions: [{ kind: "previous_run_finished" }] },
      state: "proposed", result_id: null, replaced_by: null, error: null, created_at: clock.ago(31), updated_at: clock.ago(31) },
  ];
  state.proposals.push(...proposals);

  // A static Git view for the docs session (its folder is the checkout).
  state.sessionGit["demo-s-docs"] = {
    branch: "lp/document-refund-flow", committed: false, merged_into: null, commits: [],
    files: [{ path: "guides/refunds.md", status: "??", previous: null }],
    patches: { "guides/refunds.md": gitPatch("guides/refunds.md", "@@ -0,0 +1,12 @@\n+# Refunds\n+\n+A refund moves through four states:\n+\n+```mermaid\n+stateDiagram-v2\n+  Requested --> Approved\n+  Approved --> Sent\n+  Approved --> Failed\n+  Failed --> Approved: retry\n+```\n+\n+Only **billing admins** can approve a refund above 500 credits.\n", "add") },
  };

  // Scheduled: one launch that waits for the Claude 5-hour quota reset.
  const schedules: ScheduledLaunch[] = [{
    id: "demo-sched-guide", title: "Write the v2 API migration guide", created_at: clock.ago(50), status: "queued", error: null, resume_session_id: null,
    spec: defaultSpec({ harness: "claude", profile: "claude-max", target: { kind: "project", project_id: P.docs }, worktree: { base: null, include_changes: false, merge_when_done: false }, text: "Write the v2 API migration guide: every breaking change with a before/after example.", workspace_id: WS.acme }),
    conditions: [{ kind: "quota", harness: "claude", window: "five_hour" }] satisfies LaunchDelay[],
    ready_at: clock.ago(-QUOTA_RESET_MINUTES),
  }];

  // Cron: the nightly dependency audit (last run: demo-s-audit).
  const crons: CronAgent[] = [{
    id: "demo-cron-audit", title: "Nightly dependency audit", schedule: "0 3 * * *", paused: false, created_at: clock.ago(60 * 24 * 12),
    spec: defaultSpec({ harness: "codex", profile: "codex-plus", target: { kind: "project", project_id: P.billing }, text: "Run cargo audit and pnpm audit. Report new advisories only, with the fix for each.", workspace_id: WS.acme }),
    conditions: [{ kind: "quota_left", window: "weekly", min_left_percent: 30 }, { kind: "previous_run_finished" }],
    next_run_at: nextAt(3, 0), last_check_at: clock.ago(60 * 9), last_outcome: "Started Nightly dependency audit", runs: ["demo-s-audit"],
  }];

  return {
    ...state,
    schedules,
    crons,
    /** Composer drafts per workspace key (`/api/launch/draft`), with their revision. */
    drafts: {} as Record<string, Record<string, unknown> & { revision: number }>,
    /** Uploaded attachments (`POST /api/uploads`), by content hash. */
    uploads: {} as Record<string, FileAttachment & { blob: Blob }>,
  };
}

/** The next local time `hour:minute` (epoch ms, demo clock). */
function nextAt(hour: number, minute: number): number {
  const d = new Date(clock.now());
  d.setHours(hour, minute, 0, 0);
  if (d.getTime() <= clock.now()) d.setDate(d.getDate() + 1);
  return d.getTime();
}
