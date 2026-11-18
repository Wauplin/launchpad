// Folios part of the demo world: the storefront launch plan, written by the Maintainer's documentation session, with
// a few of your marks (verdicts, a choice, an unsent comment) and an agent's reply. Home's check finds no problem in
// it; the demo routes (../routes/folios.ts) compute outlines with the UI's splitter.

import type { Folio } from "@/gen/Folio";
import type { FolioMark } from "@/gen/FolioMark";
import type { FolioRev } from "@/gen/FolioRev";
import type { Author } from "@/gen/Author";
import { clock } from "../daemon";
import { WS } from "./ids";

export type DemoFolio = { folio: Folio; files: Record<string, string>; marks: FolioMark[]; revs: FolioRev[]; data?: Record<string, unknown> };

const INDEX = `---
title: Launchpad roadmap, winter 2026
kind: roadmap
status: for review
summary: Three waves to ship Folios, the store and reader first.
pills: ["Feasible", "≈ 9 weeks"]
pages: [index.md, plan.md]
---

:::callout{tone=verdict} Verdict
Ship the store and reader first. Comments come second, because they need stable anchors. Sharing comes last.
:::

\`\`\`metrics
items:
  - { label: Static Spaces built, value: "35", text: Oct 4 to Nov 12 }
  - { label: Text in a report, value: 39 %, tone: warn, text: the rest is CSS and scripts }
  - { label: Output tokens saved, value: ≈ 2.5×, tone: good }
\`\`\`

## Waves {#waves}

\`\`\`roadmap
waves:
  - id: W1
    title: Store, CLI and reader
    size: M
    items:
      - { id: F-01, title: Plain store and records, size: S, advice: do, status: done }
      - { id: F-02, title: Folio tab in sessions, deps: [F-01], advice: do, status: doing }
      - { id: F-03, title: Shared snapshots, advice: later }
  - id: W2
    title: Comments and decisions
    size: M
    items:
      - { id: F-07, title: Anchored comments, deps: [F-01], advice: discuss }
\`\`\`

## Decisions {#decisions}

\`\`\`decision
id: D-SHARE
question: Should shared snapshots expire?
why: Snapshots live on the Hub. Without an expiry they pile up like the static Spaces did.
options:
  - { id: after-30-days, label: After 30 days, text: You can extend it. Nothing lingers., recommended: true }
  - { id: never, label: Never, text: Stays until Stop sharing. }
\`\`\`

:::scope
### In
- The store, the CLI and the reader
- Marks and feedback
### Out
- Public live sharing
:::

> [!NOTE]
> Every write is checked: file, line, rule and a fix.

## Sources {#sources}

\`\`\`sources
items:
  - { title: markdown-it, url: "https://github.com/markdown-it/markdown-it", note: the renderer }
  - { title: Hypothesis anchoring, url: "https://web.hypothes.is", note: quotes that survive edits }
\`\`\`
`;

const PLAN = `# Plan: three waves

Each wave ships on its own. See the [waves](index.md#waves) and the [checks](#checks).

## Steps {#steps}

\`\`\`steps
items:
  - { title: Write, text: The agent runs \`launchpad folio create\` and writes pages. }
  - { title: Read, text: You open it from the session's Folio tab or Folios. }
  - { title: Mark, text: "You comment, pick options and set verdicts." }
  - { title: Send, text: Send feedback messages the folio's thread. }
\`\`\`

## Status {#status}

\`\`\`status
tiles:
  - { area: Store, state: done, text: Writes and revs work }
  - { area: Reader, state: warn, text: Phone layout under review }
  - { area: Sharing, state: todo, text: Phase 5 }
\`\`\`

## Checks {#checks}

\`\`\`checklist
items:
  - { id: C-1, text: Read the plan }
  - { id: C-2, text: Pick the snapshot expiry }
  - { id: C-3, text: Name the feature, done: true }
\`\`\`

:::tabs
### Laptop
The reader has the contents on the left and comments on the right.
### Phone
The same folio in the phone shell, with a sheet for comments.
:::

\`\`\`table
caption: Report sizes
columns: [Space, KB, Text]
rows:
  - [oracle design report, 171, 39 %]
  - [full audit, 113, 34 %]
\`\`\`

## Sketches {#sketches}

\`\`\`sketch
id: S-INBOX
title: The inbox on a phone (fake data)
src: sketches/inbox.html
frame: phone
height: 420
\`\`\`

\`\`\`chart
id: CH-SIZES
title: Report sizes, HTML and text
type: bar
unit: KB
columns: [report, html, text]
rows:
  - [oracle design, 171, 66]
  - [full audit, 113, 41]
  - [exit routes, 138, 52]
\`\`\`

\`\`\`mermaid
flowchart LR
  A[Agent writes] --> B[Home checks]
  B --> C[You mark]
  C --> D[Send feedback]
  D --> A
\`\`\`

\`\`\`prompt
id: P-W1
title: Prompt for the implementer
text: |
  Implement wave 1 of the folio roadmap-winter. Follow the decisions recorded there.
\`\`\`
`;

const INBOX = `<div class="lp-bar">Inbox <span class="lp-pill attn">3</span></div>
<div id="rows"></div>
<p class="lp-muted" id="note">Tap a row to mark it read: it is saved with folio.data.</p>
<script>
const items = ["Refund flow review", "Storefront launch plan", "Docs for checkout"];
async function draw() {
  const read = (await folio.data.get("inbox:read").catch(() => null)) || [];
  document.getElementById("rows").innerHTML = items.map((t, i) => '<div class="lp-row" data-i="' + i + '"><span class="lp-grow">' + t + '</span>' + (read.includes(i) ? '<span class="lp-chip on">read</span>' : '<span class="lp-chip">new</span>') + '</div>').join("");
}
document.getElementById("rows").addEventListener("click", async (e) => {
  const row = e.target.closest("[data-i]"); if (!row) return;
  const read = (await folio.data.get("inbox:read").catch(() => null)) || [];
  const i = Number(row.dataset.i);
  await folio.data.set("inbox:read", read.includes(i) ? read.filter((x) => x !== i) : [...read, i]);
  draw();
});
draw();
</script>`;

export const FOLIO_SESSION = "demo-s-docs";

export function foliosWorld() {
  const agent: Author = { kind: "agent", name: "Document the refund flow", session_id: FOLIO_SESSION, model: "claude-opus-5-5", runner: "laptop" };
  const you: Author = { kind: "owner", name: "You", session_id: null, model: null, runner: null };
  const mark = (m: Partial<FolioMark> & Pick<FolioMark, "id" | "kind">): FolioMark => ({ seq: m.id, author: you, page: "index.md", target: null, quote: null, prefix: null, suffix: null, value: null, text: null, parent: null, rev: 3, created_at: clock.ago(40), updated_at: clock.ago(40), sent_at: null, resolved_at: null, resolved_rev: null, ...m });
  const folio: Folio = {
    id: "demo-folio-winter", workspace_id: WS.acme, slug: "roadmap-winter", title: "Launchpad roadmap, winter 2026", kind: "roadmap", state: "active",
    created_by: agent, created_at: clock.ago(60 * 5), updated_at: clock.ago(50), rev: 3, topic_id: "demo-topic-launch", thread: null, archived_at: null,
    errors: 0, warnings: 0, status: "for review", summary: "Three waves to ship Folios, the store and reader first.", unsent: 0, open: 0,
    links: [{ session_id: FOLIO_SESSION, role: "created", first_rev: 1, last_rev: 3, at: clock.ago(50), title: "Document the refund flow" }],
    share: { space: "you/folio-roadmap-winter", url: "https://huggingface.co/spaces/you/folio-roadmap-winter", rev: 2, visibility: "private", expires_at: clock.ago(-60 * 24 * 21), decisions: false, at: clock.ago(60 * 24 * 9), state: "shared", error: null, dry_run: false },
  };
  const marks: FolioMark[] = [
    mark({ id: 1, kind: "verdict", target: "F-01", value: "do", sent_at: clock.ago(45) }),
    mark({ id: 2, kind: "comment", target: "F-03", quote: "Shared snapshots", text: "Move this to wave 3: sharing comes last.", sent_at: clock.ago(45), resolved_at: clock.ago(50), resolved_rev: 3 }),
    mark({ id: 3, kind: "reply", author: agent, parent: 2, target: "F-03", text: "Done: F-03 is advised for later, and the verdict says why.", sent_at: clock.ago(50), created_at: clock.ago(50) }),
    mark({ id: 4, kind: "verdict", target: "F-03", value: "later" }),
    mark({ id: 5, kind: "choice", target: "D-SHARE", value: "after-30-days" }),
    mark({ id: 6, kind: "comment", target: "waves", quote: "Comments come second, because they need stable anchors", prefix: "reader first. ", suffix: ". Sharing", text: "Could we ship page-level notes first, without anchors?" }),
  ];
  const w = (path: string, before: string | null, after: string, added: number, removed: number) => ({ path, before, after, added, removed });
  const revs: FolioRev[] = [
    { rev: 3, at: clock.ago(50), author: agent, session_id: FOLIO_SESSION, label: null, files: [w("index.md", "demo-index-2", "demo-index-3", 2, 1)], errors: 0, warnings: 0, version: null },
    { rev: 2, at: clock.ago(60 * 4), author: agent, session_id: FOLIO_SESSION, label: null, files: [w("plan.md", null, "demo-plan-2", PLAN.split("\n").length, 0)], errors: 0, warnings: 0, version: null },
    { rev: 1, at: clock.ago(60 * 5), author: agent, session_id: FOLIO_SESSION, label: "created", files: [w("index.md", null, "demo-index-1", INDEX.split("\n").length, 0)], errors: 0, warnings: 0, version: "v1 · first draft" },
  ];
  return { folios: [{ folio, files: { "index.md": INDEX, "plan.md": PLAN, "sketches/inbox.html": INBOX }, marks, revs }] as DemoFolio[] };
}
