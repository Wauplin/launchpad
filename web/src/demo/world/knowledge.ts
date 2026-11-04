// Knowledge part of the demo world: the Acme knowledge base with notes by agents and by you, a concurrent edit
// and the memory files a runner would offer for import.

import type { Change } from "@/gen/Change";
import type { NativeMemory } from "@/gen/NativeMemory";
import { clock } from "../daemon";
import { WS } from "./ids";

export type DemoNote = { path: string; content: string; sha256: string };
export type DemoKnowledge = { notes: DemoNote[]; changes: (Change & { patch: string })[]; seen: number; next: number };

let n = 0;
export const demoSha = () => (++n).toString(16).padStart(64, "0");

const note = (path: string, content: string): DemoNote => ({ path, content, sha256: demoSha() });

const INDEX = `# Index

- [[checkout-flow]]: how checkout calls the payment provider, and its flaky test
- [[stripe-webhooks]]: why webhooks are verified before the order is saved
- [[maria]]: owns the billing API
- [[release]]: how to cut a release of the storefront
`;
const CHECKOUT = `---
title: Checkout flow
type: fact
status: current
valid_from: 2026-10-30
sources:
  - trace: demo-s-checkout#42
---
Checkout creates a payment intent, then waits for the \`payment_intent.succeeded\` webhook before it marks the order paid. See [[stripe-webhooks]].

The end-to-end test was flaky because it did not wait for the webhook: it now polls the order for 10 seconds.
`;
const WEBHOOKS = `---
title: Verify webhooks first
type: decision
status: current
valid_from: 2026-10-12
---
Webhooks are verified with the signing secret before anything is saved. An unverified event is answered with 400 and logged. Asked by [[maria]] after the October incident.
`;

export function knowledgeWorld() {
  const notes = [
    note("INDEX.md", INDEX),
    note("AGENTS.md", "# How to write here\n\nOne note per subject in people/, projects/, topics/, decisions/ or howto/, with front matter. Link notes with [[wikilinks]] and keep INDEX.md current.\n"),
    note("topics/checkout-flow.md", CHECKOUT),
    note("decisions/stripe-webhooks.md", WEBHOOKS),
    note("people/maria.md", "---\ntitle: Maria\ntype: person\n---\nOwns the billing API. Prefers small pull requests with a test.\n"),
    note("howto/release.md", "---\ntitle: Release the storefront\ntype: howto\n---\n1. Merge to main.\n2. Tag `v<version>`.\n3. CI deploys the tag.\n"),
    note("bots/maintainer/checkout.md", "---\ntitle: Checkout, as the Maintainer sees it\ntype: project\nstatus: current\n---\n- Webhooks are verified before the order is saved (7f03b11).\n- The e2e test polls the order for 10 seconds (a41c9e2).\n- Risk: the refund path has no end-to-end test yet. The owner may ask for one.\n"),
    note("howto/release.concurrent-3fa94c1e.md", "---\ntitle: Release the storefront\ntype: howto\n---\n1. Merge to main and wait for CI.\n2. Tag `v<version>` and push the tag.\n3. CI deploys the tag; check the dashboard.\n"),
  ];
  const agent = (session: string, title: string, model: string) => ({ kind: "agent" as const, name: title, session_id: session, model, runner: "laptop" });
  const you = { kind: "owner" as const, name: "You", session_id: null, model: null, runner: null };
  const change = (id: number, path: string, minutes: number, author: Change["author"], added: number, removed: number, patch: string, created = false): Change & { patch: string } =>
    ({ id, path, sha256: created ? null : demoSha(), new_sha256: notes.find((x) => x.path === path)?.sha256 ?? demoSha(), at: clock.ago(minutes), session_id: author?.session_id ?? null, author, added, removed, patch });
  const changes = [
    change(8, "bots/maintainer/checkout.md", 66, agent("demo-s-bot-maintainer", "What changed in checkout this week", "demo-large"), 7, 0, "@@ -0,0 +1,7 @@\n+---\n+title: Checkout, as the Maintainer sees it\n+type: project\n+status: current\n+---\n+- Webhooks are verified before the order is saved (7f03b11).\n+- Risk: the refund path has no end-to-end test yet.\n", true),
    change(7, "howto/release.concurrent-3fa94c1e.md", 25, agent("demo-s-node24", "Upgrade CI to Node 24", "gpt-5.5"), 6, 0, "@@ -0,0 +1,6 @@\n+---\n+title: Release the storefront\n+type: howto\n+---\n+1. Merge to main and wait for CI.\n+2. Tag `v<version>` and push the tag.\n", true),
    change(6, "topics/checkout-flow.md", 40, agent("demo-s-checkout", "Fix the flaky checkout test", "claude-opus-5-5"), 1, 1, "@@ -10,3 +10,3 @@\n Checkout creates a payment intent, then waits for the `payment_intent.succeeded` webhook before it marks the order paid. See [[stripe-webhooks]].\n \n-The end-to-end test is flaky.\n+The end-to-end test was flaky because it did not wait for the webhook: it now polls the order for 10 seconds.\n"),
    change(5, "INDEX.md", 41, agent("demo-s-checkout", "Fix the flaky checkout test", "claude-opus-5-5"), 1, 0, "@@ -1,3 +1,4 @@\n # Index\n \n+- [[checkout-flow]]: how checkout calls the payment provider, and its flaky test\n - [[stripe-webhooks]]: why webhooks are verified before the order is saved\n"),
    change(4, "people/maria.md", 60 * 20, you, 4, 0, "@@ -0,0 +1,4 @@\n+---\n+title: Maria\n+type: person\n+---\n", true),
    change(3, "decisions/stripe-webhooks.md", 60 * 26, agent("demo-s-invoices", "Rate-limit /invoices", "claude-opus-5-5"), 7, 0, "@@ -0,0 +1,7 @@\n+---\n+title: Verify webhooks first\n+type: decision\n+status: current\n+valid_from: 2026-10-12\n+---\n+Webhooks are verified with the signing secret before anything is saved.\n", true),
  ];
  const memories: NativeMemory[] = [
    { path: "imported/claude-code/projects-storefront/MEMORY.md", source: "/home/demo/.claude/projects/-home-demo-projects-storefront/memory/MEMORY.md", content: "# Memory\n- [Testing](testing.md): run pnpm test:e2e before pushing\n" },
    { path: "imported/claude-code/projects-storefront/testing.md", source: "/home/demo/.claude/projects/-home-demo-projects-storefront/memory/testing.md", content: "Run pnpm test:e2e before pushing; it needs the Stripe CLI.\n" },
    { path: "imported/codex/memories/MEMORY.md", source: "/home/demo/.codex/memories/MEMORY.md", content: "The billing API uses sqlx offline mode in CI.\n" },
  ];
  const knowledge: Record<string, DemoKnowledge> = {
    [WS.acme]: { notes, changes, seen: 4, next: 9 },
    [WS.personal]: { notes: [note("INDEX.md", "# Index\n"), note("AGENTS.md", "# How to write here\n")], changes: [], seen: 0, next: 1 },
  };
  return { knowledge, memories };
}
