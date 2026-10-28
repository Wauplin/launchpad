// Chapter 6: Ask Launchpad "What needs me today?". The assistant reads the workspace with the read-only CLI, answers,
// and writes a launch proposal; nothing starts until the owner presses Start.

import type { Proposal } from "@/gen/Proposal";
import { clock, publish } from "../daemon";
import { at, effect, final, scenario, think, tool } from "../player";
import { defaultSpec } from "../store";
import { P } from "../world/ids";

export const TODAY_PROPOSAL = "demo-prop-today";

export default scenario({
  id: "ask-today",
  folder: "storefront",
  harness: "claude",
  title: "What needs me today?",
  timeline: [
    at(0.4, think("The owner wants a short list of what needs them. I will read the agents, the inbox and the schedules of this workspace.")),
    at(0.8, tool("bash", "launchpad sessions list --workspace Acme", { seconds: 0.9, output: `[{"title":"Rate-limit /invoices","status":"waiting","needs_you":true,"open_question":true},
 {"title":"Fix the flaky checkout test","status":"waiting","needs_you":true},
 {"title":"Upgrade CI to Node 24","status":"error","needs_you":true},
 {"title":"Split the invoice service into modules","status":"quota_paused"}, …]` })),
    at(0.6, tool("bash", "launchpad inbox list --workspace Acme", { seconds: 0.9, output: `[{"repo":"acme-labs/storefront","number":118,"reason":"review_requested","title":"Add Apple Pay to checkout"},
 {"repo":"acme-labs/billing-api","number":77,"reason":"mention","title":"Invoices API returns 500 on empty cart"}, …]` })),
    at(0.6, tool("bash", "launchpad schedules list --workspace Acme", { seconds: 0.6, output: `[{"title":"Write the v2 API migration guide","conditions":["quota available"]}]` })),
    at(0.8, think("The failed CI upgrade can be retried with the lockfile fix. I will propose that launch; the owner decides.")),
    at(0.6, tool("bash", "launchpad propose launch --title \"Retry the Node 24 CI upgrade\" --spec-file /tmp/ci.json", { seconds: 0.8, output: "Proposal created: Retry the Node 24 CI upgrade (waiting for the owner)" })),
    at(0, effect((c) => {
      if (c.w.proposals.some((p) => p.id === TODAY_PROPOSAL && p.state === "proposed")) return;
      const s = c.w.sessions.find((x) => x.id === c.sessionId);
      const target = s?.launch_spec.target;
      const workspace = target?.kind === "assistant" ? target.workspace_id : null;
      const now = clock.now();
      const p: Proposal = {
        id: TODAY_PROPOSAL, workspace_id: workspace, session_id: c.sessionId, title: "Retry the Node 24 CI upgrade",
        note: "The CI upgrade failed on `pnpm install` because the lockfile was written by an older pnpm. A new run that regenerates it should pass.",
        body: { kind: "launch", spec: defaultSpec({ harness: "codex", profile: "codex-plus", target: { kind: "project", project_id: P.storefront }, worktree: { base: null, include_changes: false, merge_when_done: false }, text: "Upgrade CI to Node 24: regenerate pnpm-lock.yaml with pnpm 10, then make the CI workflow pass.", workspace_id: workspace }) },
        state: "proposed", result_id: "demo-s-proposed-ci", replaced_by: null, error: null, created_at: now, updated_at: now,
      };
      c.w.proposals = [...c.w.proposals.filter((x) => x.id !== TODAY_PROPOSAL), p];
      if (c.live) publish("proposals", "changed", p);
    })),
    final(`Here is what needs you today in **Acme**:

1. **Rate-limit /invoices** asks a question: where the request counter should live.
2. **Fix the flaky checkout test** is done and not reviewed yet.
3. **Upgrade CI to Node 24** failed on \`pnpm install\`.
4. **PR #118 Add Apple Pay to checkout** waits for your review.

*Split the invoice service into modules* is quota paused and continues by itself after the reset.

I proposed one launch: **Retry the Node 24 CI upgrade**. Press **Start** on the card to run it, or **Edit** to change it first.`),
  ],
});
