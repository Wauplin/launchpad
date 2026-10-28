// GitHub part of the demo world (inbox, issues, PRs, Local PRs, suggestions, review state). Never contacted:
// fake data only. Items use the GraphQL shape of `core/items.rs` (`features/github/types.ts`).
//
// For the scenario player: `addLocalPr`, `addSuggestion` and `finishIssue42` (below) add a Local PR and an
// agent-written suggestion at the end of the "Draft a Local PR for issue #42" run and publish the bus events,
// so Review, the folder page and the PR page update like with the real daemon.

import type { Comment, Item, Label, User } from "@/features/github/types";
import type { Note } from "@/gen/Note";
import type { Predraft } from "@/gen/Predraft";
import type { Suggestion } from "@/gen/Suggestion";
import type { SuggestionKind } from "@/gen/SuggestionKind";
import type { Thread } from "@/gen/Thread";
import { clock, publish } from "../daemon";
import { world } from "./index";
import { DEMO_COMMITS, WT, type Change } from "./code";
import { ORG, P, ROOT, USER } from "./ids";

export const REPO = { storefront: `${ORG}/storefront`, billing: `${ORG}/billing-api`, docs: `${ORG}/docs` } as const;

// ---------------------------------------------------------------- helpers

const iso = (minutesAgo: number) => new Date(clock.ago(minutesAgo)).toISOString();
const user = (login: string): User => ({ login });
const escape = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

/** A small Markdown subset to GitHub-like HTML: paragraphs, `- ` lists, `code`, **bold** and fenced blocks. */
export function mdToHtml(text: string): string {
  const inline = (s: string) => escape(s).replace(/`([^`]+)`/g, "<code>$1</code>").replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
  return text.trim().split(/\n{2,}/).map((block) => {
    if (block.startsWith("```")) return `<pre><code>${escape(block.replace(/^```\w*\n?/, "").replace(/\n?```$/, ""))}</code></pre>`;
    const lines = block.split("\n");
    if (lines.every((l) => /^[-*] /.test(l))) return `<ul>${lines.map((l) => `<li>${inline(l.slice(2))}</li>`).join("")}</ul>`;
    if (lines.every((l) => /^\d+\. /.test(l))) return `<ol>${lines.map((l) => `<li>${inline(l.replace(/^\d+\. /, ""))}</li>`).join("")}</ol>`;
    if (/^#{1,6} /.test(block)) { const level = block.match(/^#+/)![0].length; return `<h${level}>${inline(block.slice(level + 1))}</h${level}>`; }
    return `<p>${lines.map(inline).join("<br>")}</p>`;
  }).join("\n");
}

/** A 40-character hex ID derived from a text, for new commits and heads. */
export function fakeSha(seed: string): string {
  let h = 2166136261;
  let out = "";
  for (let round = 0; out.length < 40; round++) {
    for (const c of `${seed}:${round}`) h = Math.imul(h ^ c.charCodeAt(0), 16777619);
    out += (h >>> 0).toString(16).padStart(8, "0");
  }
  return out.slice(0, 40);
}

// ---------------------------------------------------------------- labels and people

const L = {
  bug: { name: "bug", color: "d73a4a" },
  enhancement: { name: "enhancement", color: "a2eeef" },
  tests: { name: "tests", color: "0e8a16" },
  frontend: { name: "frontend", color: "1d76db" },
  goodFirst: { name: "good first issue", color: "7057ff" },
  deps: { name: "dependencies", color: "0366d6" },
  security: { name: "security", color: "b60205" },
  api: { name: "api", color: "fbca04" },
  docs: { name: "documentation", color: "0075ca" },
  question: { name: "question", color: "d876e3" },
} satisfies Record<string, Label>;

/** Labels and possible reviewers per repository (`/api/repo-meta`). */
export const repoMeta: Record<string, { labels: Label[]; reviewers: string[] }> = {
  [REPO.storefront]: { labels: [L.bug, L.enhancement, L.tests, L.frontend, L.goodFirst, L.deps], reviewers: ["dev-alex", "dev-kim", "dev-rui", "team:frontend"] },
  [REPO.billing]: { labels: [L.bug, L.enhancement, L.security, L.api], reviewers: ["dev-sam", "dev-rui", "team:billing"] },
  [REPO.docs]: { labels: [L.docs, L.goodFirst, L.question], reviewers: ["dev-kim", "dev-alex", "team:docs"] },
};

// ---------------------------------------------------------------- items

type Check = { name: string; conclusion: "SUCCESS" | "FAILURE" | null; status: "COMPLETED" | "IN_PROGRESS" };
export type ItemSpec = {
  repo: string; number: number; pr?: boolean; title: string; author: string; body: string; labels: Label[]; minutesAgo: number; updatedAgo?: number;
  state?: "OPEN" | "CLOSED" | "MERGED"; draft?: boolean; head?: string; base?: string; branch?: string; mergeable?: string; mergeState?: string; reviewDecision?: string | null;
  checks?: Check[]; comments?: { author: string; body: string; minutesAgo: number }[]; reviews?: { author: string; state: string; body: string; minutesAgo: number }[];
  requested?: string[]; assignees?: string[]; threads?: { path: string; line: number; resolved?: boolean; comments: { author: string; body: string; minutesAgo: number }[] }[];
  commits?: { subject: string; minutesAgo: number }[]; closes?: number[];
};

/** Builds an item in the GraphQL shape of `core/items.rs`. */
export function buildItem(s: ItemSpec): Item {
  const url = `https://github.com/${s.repo}/${s.pr ? "pull" : "issues"}/${s.number}`;
  const id = `${s.pr ? "PR" : "I"}_${s.repo.replace(/\W/g, "")}${s.number}`;
  const comment = (c: { author: string; body: string; minutesAgo: number }, i: number): Comment => ({ id: `${id}_c${i}`, body: c.body, bodyHTML: mdToHtml(c.body), url: `${url}#issuecomment-${i}`, createdAt: iso(c.minutesAgo), author: user(c.author) });
  const timeline: Item["timelineItems"]["nodes"] = [];
  (s.commits ?? []).forEach((c, i) => timeline.push({ __typename: "PullRequestCommit", id: `${id}_k${i}`, commit: { oid: fakeSha(`${id}${i}`), messageHeadline: c.subject, committedDate: iso(c.minutesAgo), url: `${url}/commits/${fakeSha(`${id}${i}`)}`, author: user(s.author) } }));
  (s.requested ?? []).forEach((r, i) => timeline.push({ __typename: "ReviewRequestedEvent", id: `${id}_r${i}`, createdAt: iso(s.minutesAgo - 1), actor: user(s.author), requestedReviewer: r.startsWith("team:") ? { name: r.slice(5), slug: r.slice(5) } : user(r) }));
  s.labels.forEach((l, i) => timeline.push({ __typename: "LabeledEvent", id: `${id}_l${i}`, createdAt: iso(s.minutesAgo - 2), actor: user(s.author), label: { name: l.name } }));
  (s.comments ?? []).forEach((c, i) => timeline.push({ __typename: "IssueComment", ...comment(c, i) }));
  (s.reviews ?? []).forEach((r, i) => timeline.push({ __typename: "PullRequestReview", id: `${id}_v${i}`, body: r.body, bodyHTML: mdToHtml(r.body), url: `${url}#pullrequestreview-${i}`, submittedAt: iso(r.minutesAgo), createdAt: iso(r.minutesAgo), author: user(r.author), state: r.state }));
  if (s.state === "MERGED") timeline.push({ __typename: "MergedEvent", id: `${id}_m`, createdAt: iso(s.updatedAgo ?? 60), actor: user("dev-sam") });
  if (s.state === "CLOSED") timeline.push({ __typename: "ClosedEvent", id: `${id}_x`, createdAt: iso(s.updatedAgo ?? 60), actor: user(s.author) });
  timeline.sort((a, b) => (a.createdAt ?? a.submittedAt ?? a.commit?.committedDate ?? "").localeCompare(b.createdAt ?? b.submittedAt ?? b.commit?.committedDate ?? ""));
  const item: Item = {
    __typename: s.pr ? "PullRequest" : "Issue", id, repo: s.repo, number: s.number, title: s.title, url, body: s.body, bodyHTML: mdToHtml(s.body),
    state: s.state === "MERGED" ? "MERGED" : s.state ?? "OPEN", createdAt: iso(s.minutesAgo), updatedAt: iso(s.updatedAgo ?? Math.min(s.minutesAgo, 30)), fetched_at: clock.now(),
    author: user(s.author), labels: { nodes: s.labels }, assignees: { nodes: (s.assignees ?? []).map(user) },
    participants: { nodes: [...new Set([s.author, ...(s.comments ?? []).map((c) => c.author), ...(s.reviews ?? []).map((r) => r.author)])].map(user) },
    timelineItems: { nodes: timeline },
  };
  if (s.pr) {
    const head = s.head ?? fakeSha(`${id}:head`);
    Object.assign(item, {
      isDraft: !!s.draft, merged: s.state === "MERGED", mergeable: s.mergeable ?? "MERGEABLE", mergeStateStatus: s.mergeState ?? "CLEAN", reviewDecision: s.reviewDecision ?? "REVIEW_REQUIRED",
      headRefName: s.branch ?? `topic-${s.number}`, baseRefName: "main", headRefOid: head, baseRefOid: s.base ?? fakeSha(`${id}:base`),
      repository: { mergeCommitAllowed: true, squashMergeAllowed: true, rebaseMergeAllowed: false },
      reviewRequests: { nodes: (s.requested ?? []).map((r) => ({ requestedReviewer: r.startsWith("team:") ? { name: r.slice(5), slug: r.slice(5) } : user(r) })) },
      reviews: { nodes: (s.reviews ?? []).map((r) => ({ state: r.state, author: user(r.author) })) },
      reviewThreads: { nodes: (s.threads ?? []).map((t, i) => ({ id: `${id}_t${i}`, path: t.path, line: t.line, isResolved: !!t.resolved, isOutdated: false, comments: { nodes: t.comments.map((c, j) => comment(c, 100 + i * 10 + j)) } })) },
      closingIssuesReferences: { nodes: (s.closes ?? []).map((n) => ({ number: n, title: `Issue #${n}`, url: `https://github.com/${s.repo}/issues/${n}` })) },
      commits: { nodes: [{ commit: { statusCheckRollup: s.checks?.length ? {
        state: s.checks.some((c) => c.conclusion === "FAILURE") ? "FAILURE" : s.checks.some((c) => c.status !== "COMPLETED") ? "PENDING" : "SUCCESS",
        contexts: { nodes: s.checks.map((c) => ({ name: c.name, conclusion: c.conclusion ?? undefined, status: c.status, detailsUrl: `https://github.com/${s.repo}/actions` })) },
      } : undefined } }] },
    });
  }
  return item;
}

const green: Check[] = [{ name: "test", conclusion: "SUCCESS", status: "COMPLETED" }, { name: "lint", conclusion: "SUCCESS", status: "COMPLETED" }, { name: "build", conclusion: "SUCCESS", status: "COMPLETED" }];

/** The file changes of each demo PR (`/diff`). */
function prChanges(): Record<string, Change[]> {
  const trees = world.codeTrees;
  const sf = trees[P.storefront], bl = trees[P.billing], dc = trees[P.docs];
  return {
    [`${REPO.storefront}#118`]: [
      { path: "src/savedCarts.ts", before: null, after: `import type { Cart } from "./cart";

const KEY = "storefront.saved-carts";

/** Carts the visitor saved for later, newest first. */
export function savedCarts(): Cart[] {
  return JSON.parse(localStorage.getItem(KEY) ?? "[]") as Cart[];
}

export function saveCart(cart: Cart) {
  const all = savedCarts().filter((c) => c.id !== cart.id);
  localStorage.setItem(KEY, JSON.stringify([cart, ...all].slice(0, 5)));
}
` },
      { path: "src/components/CartView.tsx", before: sf["src/components/CartView.tsx"], after: sf["src/components/CartView.tsx"].replace(`import { formatMoney } from "../lib/format";`, `import { formatMoney } from "../lib/format";\nimport { saveCart } from "../savedCarts";`).replace(`      <strong>Total: {formatMoney(cartTotal(cart))}</strong>`, `      <strong>Total: {formatMoney(cartTotal(cart))}</strong>\n      <button onClick={() => saveCart(cart)}>Save for later</button>`) },
      { path: "tests/savedCarts.test.ts", before: null, after: `import { expect, it } from "vitest";
import { saveCart, savedCarts } from "../src/savedCarts";

it("keeps the five newest carts", () => {
  for (let i = 0; i < 7; i++) saveCart({ id: \`c\${i}\`, lines: [] });
  expect(savedCarts().map((c) => c.id)).toEqual(["c6", "c5", "c4", "c3", "c2"]);
});
` },
    ],
    [`${REPO.storefront}#119`]: [{ path: "src/components/CartView.tsx", before: sf["src/components/CartView.tsx"], after: sf["src/components/CartView.tsx"].replace(`<span>{line.name} × {line.quantity}</span>`, `<img src={\`/img/\${line.sku}.webp\`} loading="lazy" width={48} height={48} alt="" />\n          <span>{line.name} × {line.quantity}</span>`) }],
    [`${REPO.storefront}#121`]: [{ path: "package.json", before: sf["package.json"], after: sf["package.json"].replace(`"vitest": "^2.1.0"`, `"vitest": "^3.0.0"`) }],
    [`${REPO.billing}#57`]: [{ path: "src/webhooks.rs", before: null, after: `/// Delivers a webhook, retrying with exponential backoff (1 s, 2 s, 4 s, …) up to five times.
pub async fn deliver(client: &reqwest::Client, url: &str, body: &str) -> anyhow::Result<()> {
    let mut delay = std::time::Duration::from_secs(1);
    for attempt in 1..=5 {
        match client.post(url).body(body.to_string()).send().await {
            Ok(r) if r.status().is_success() => return Ok(()),
            _ if attempt == 5 => anyhow::bail!("webhook failed after 5 attempts"),
            _ => tokio::time::sleep(delay).await,
        }
        delay *= 2;
    }
    Ok(())
}
` }],
    [`${REPO.billing}#55`]: [{ path: "src/invoices.rs", before: bl["src/invoices.rs"], after: bl["src/invoices.rs"].replace(`pub async fn create(`, `pub async fn pdf(State(pool): State<PgPool>, Path(id): Path<i64>) -> Result<Vec<u8>, ApiError> {\n    let invoice: Invoice = sqlx::query_as("SELECT * FROM invoices WHERE id = $1").bind(id).fetch_one(&pool).await?;\n    Ok(crate::pdf::render(&invoice))\n}\n\npub async fn create(`) }],
    [`${REPO.docs}#44`]: [{ path: "guide/checkout.md", before: dc["guide/checkout.md"], after: dc["guide/checkout.md"].replace("When a payment fails, the order stays pending for 24 hours.", "When a payment fails, the order stays pending for 24 hours.\nSee [Refunds](refunds.md) to return money for a paid order.") }],
  };
}

function buildItems(): Record<string, Item> {
  const specs: ItemSpec[] = [
    {
      repo: REPO.storefront, number: 118, pr: true, title: "Save carts for later", author: "dev-alex", minutesAgo: 60 * 26, updatedAgo: 35, branch: "saved-carts",
      body: "Visitors can save up to five carts and come back to them later.\n\n- New `savedCarts.ts` module, stored in local storage\n- **Save for later** button in the cart\n- Tests for the limit of five carts\n\nCloses #115", labels: [L.enhancement, L.frontend],
      requested: [USER, "dev-kim"], assignees: ["dev-alex"], checks: green, reviewDecision: "REVIEW_REQUIRED", closes: [115],
      commits: [{ subject: "Add saved carts module", minutesAgo: 60 * 26 }, { subject: "Add Save for later button", minutesAgo: 60 * 25 }, { subject: "Keep only the five newest carts", minutesAgo: 60 * 3 }],
      comments: [
        { author: "dev-kim", body: "Nice! Should saved carts survive a sign-out? Local storage is per browser.", minutesAgo: 60 * 20 },
        { author: "dev-alex", body: "Good point. For now they stay in the browser; syncing to the account is a follow-up (#122).", minutesAgo: 60 * 19 },
      ],
      reviews: [{ author: "dev-kim", state: "COMMENTED", body: "Two small things inline, otherwise looks good.", minutesAgo: 60 * 18 }],
      threads: [
        { path: "src/savedCarts.ts", line: 7, comments: [{ author: "dev-kim", body: "`JSON.parse` throws on a corrupted value. Wrap it and return `[]`?", minutesAgo: 60 * 18 }, { author: "dev-alex", body: "Will do.", minutesAgo: 60 * 17 }] },
        { path: "src/components/CartView.tsx", line: 18, resolved: true, comments: [{ author: "dev-kim", body: "Disable the button for an empty cart.", minutesAgo: 60 * 18 }] },
      ],
    },
    {
      repo: REPO.storefront, number: 119, pr: true, title: "Lazy-load product images", author: "dev-rui", minutesAgo: 60 * 5, updatedAgo: 50, branch: "lazy-images",
      body: "Product images below the fold now load lazily. The cart page loads 40% fewer bytes on first paint.", labels: [L.frontend],
      requested: [USER], checks: [{ name: "test", conclusion: "SUCCESS", status: "COMPLETED" }, { name: "lint", conclusion: "SUCCESS", status: "COMPLETED" }, { name: "build", conclusion: null, status: "IN_PROGRESS" }],
      mergeState: "BLOCKED", commits: [{ subject: "Lazy-load cart images", minutesAgo: 60 * 5 }],
    },
    {
      repo: REPO.storefront, number: 121, pr: true, title: "Upgrade the test runner to vitest 3", author: "dev-kim", minutesAgo: 60 * 9, updatedAgo: 95, branch: "vitest-3",
      body: "Upgrades vitest. Two snapshot tests need new snapshots.\n\ncc @acme-labs/frontend", labels: [L.deps, L.tests],
      checks: [{ name: "test", conclusion: "FAILURE", status: "COMPLETED" }, { name: "lint", conclusion: "SUCCESS", status: "COMPLETED" }, { name: "build", conclusion: "SUCCESS", status: "COMPLETED" }],
      mergeState: "UNSTABLE", commits: [{ subject: "Bump vitest to 3.0", minutesAgo: 60 * 9 }],
      comments: [{ author: "dev-kim", body: "@acme-labs/frontend the checkout test fails with the new runner. Anyone seen this before?", minutesAgo: 95 }],
    },
    {
      repo: REPO.billing, number: 57, pr: true, draft: true, title: "Retry webhook deliveries", author: "dev-sam", minutesAgo: 60 * 30, updatedAgo: 60 * 4, branch: "webhook-retries",
      body: "Work in progress: retries failed webhook deliveries with exponential backoff.\n\n- [ ] Store attempts in the database\n- [ ] Metrics", labels: [L.enhancement, L.api],
      checks: green, commits: [{ subject: "Retry webhook deliveries with backoff", minutesAgo: 60 * 30 }],
    },
    {
      repo: REPO.billing, number: 55, pr: true, state: "MERGED", title: "Add invoice PDF export", author: "dev-sam", minutesAgo: 60 * 72, updatedAgo: 60 * 6, branch: "invoice-pdf",
      body: "`GET /invoices/{id}/pdf` returns the invoice as a PDF.", labels: [L.enhancement, L.api], reviewDecision: "APPROVED", checks: green,
      reviews: [{ author: "dev-rui", state: "APPROVED", body: "Looks good.", minutesAgo: 60 * 8 }], commits: [{ subject: "Render invoices as PDF", minutesAgo: 60 * 72 }],
    },
    {
      repo: REPO.docs, number: 44, pr: true, title: "Link refunds from the checkout guide", author: "dev-kim", minutesAgo: 60 * 3, updatedAgo: 70, branch: "refunds-link",
      body: "Adds a link from the failed payments section to the refunds page.", labels: [L.docs], requested: [USER], checks: [{ name: "mkdocs build", conclusion: "SUCCESS", status: "COMPLETED" }],
      commits: [{ subject: "Link refunds from checkout", minutesAgo: 60 * 3 }],
    },
    // Issues
    { repo: REPO.docs, number: 42, title: "Explain webhook retries", author: "dev-sam", minutesAgo: 60 * 48, updatedAgo: 60 * 2, labels: [L.docs, L.goodFirst], assignees: [USER],
      body: "Since billing-api#57, failed webhooks are retried. The webhooks page should say:\n\n- how many times and how often we retry\n- what counts as a failure\n- how to make an endpoint idempotent",
      comments: [{ author: "dev-kim", body: "Assigned to you, since you reviewed the retry PR.", minutesAgo: 60 * 2 }] },
    { repo: REPO.storefront, number: 120, title: "Checkout test is flaky on CI", author: "dev-rui", minutesAgo: 60 * 30, updatedAgo: 45, labels: [L.bug, L.tests], assignees: [USER],
      body: "`tests/checkout.test.ts` fails about once in ten runs with a timeout.\n\n```\nFAIL tests/checkout.test.ts > checkout > creates an order\nError: Test timed out in 5000ms.\n```",
      comments: [{ author: "dev-alex", body: "Probably the fixed `setTimeout` in the test.", minutesAgo: 60 * 28 }] },
    { repo: REPO.storefront, number: 115, title: "Support gift cards at checkout", author: "dev-kim", minutesAgo: 60 * 24 * 6, updatedAgo: 60 * 7, labels: [L.enhancement, L.frontend],
      body: "Customers ask to pay with gift cards. Accept a code in the checkout form and subtract its balance.",
      comments: [{ author: "dev-alex", body: "Saved carts (#118) first, then this.", minutesAgo: 60 * 7 }] },
    { repo: REPO.billing, number: 58, title: "Rate-limit /invoices", author: "dev-rui", minutesAgo: 60 * 20, updatedAgo: 25, labels: [L.enhancement, L.security, L.api],
      body: "A client called `GET /invoices` 3,000 times in a minute. Limit it to 10 requests per minute per API key and return 429.",
      comments: [{ author: "dev-rui", body: "@demo-user can you take this one? `src/limit.rs` has a start.", minutesAgo: 25 }] },
    { repo: REPO.billing, number: 50, title: "Invoice totals round the wrong way", author: USER, minutesAgo: 60 * 24 * 9, updatedAgo: 60 * 24, state: "CLOSED", labels: [L.bug],
      body: "Totals with half cents round down. They should round half to even.", comments: [{ author: "dev-sam", body: "Fixed in #52.", minutesAgo: 60 * 24 }] },
    { repo: REPO.docs, number: 40, title: "Add a glossary page", author: "dev-alex", minutesAgo: 60 * 24 * 12, updatedAgo: 60 * 24 * 3, labels: [L.docs, L.question],
      body: "Words like *invoice*, *payment* and *refund* mean different things to different readers. Should we add a glossary?" },
  ];
  return Object.fromEntries(specs.map((s) => [`${s.repo}#${s.number}`, buildItem(s)]));
}

// ---------------------------------------------------------------- inbox

let threadSeq = 0;
function thread(item: Item, reason: string, unread: boolean, minutesAgo: number): Thread {
  return {
    id: String(9_100_000_000 + ++threadSeq), repo: item.repo, title: item.title, kind: item.__typename, number: item.number, url: item.url, subject_path: null,
    reason, unread, local_unread: false, updated_at: iso(minutesAgo), state: threadState(item),
    participants: item.participants.nodes.map((p) => p.login ?? ""), participant_count: item.participants.nodes.length,
    review_decision: item.reviewDecision ?? null, checks: item.commits?.nodes[0]?.commit.statusCheckRollup?.state ?? null,
  };
}
/** `Thread.state` as `core/inbox.rs` derives it. */
export const threadState = (item: Item) => (item.merged ? "merged" : item.isDraft && item.state === "OPEN" ? "draft" : item.state.toLowerCase());

// ---------------------------------------------------------------- Local PRs and suggestions

const predraft = (p: Partial<Predraft> & Pick<Predraft, "id" | "project_id" | "root" | "branch" | "title" | "body">): Predraft => ({
  base_branch: "main", initial_base: "", session_id: null, notes: [], archived: false, published_url: null, published_number: null,
  default_remote_branch: p.branch.replace(/^lp\//, ""), remote_branch: null, pushed_head: null, created_at: clock.ago(60), merged: null, ...p,
});

export function githubWorld() {
  threadSeq = 0;
  const ghItems = buildItems();
  const it = (repo: string, n: number) => ghItems[`${repo}#${n}`];
  const inboxThreads: Thread[] = [
    thread(it(REPO.storefront, 118), "review_requested", true, 35),
    thread(it(REPO.storefront, 119), "review_requested", true, 50),
    thread(it(REPO.docs, 44), "review_requested", false, 70),
    thread(it(REPO.billing, 58), "mention", true, 25),
    thread(it(REPO.storefront, 121), "team_mention", false, 95),
    thread(it(REPO.storefront, 120), "assign", true, 45),
    thread(it(REPO.docs, 42), "assign", false, 120),
    thread(it(REPO.billing, 57), "subscribed", false, 240),
    thread(it(REPO.billing, 55), "state_change", true, 360),
    thread(it(REPO.storefront, 115), "comment", false, 420),
    thread(it(REPO.billing, 50), "author", false, 60 * 24),
    { id: String(9_100_000_000 + ++threadSeq), repo: REPO.storefront, title: "v2.4.0", kind: "Release", number: null, url: `https://github.com/${REPO.storefront}/releases/tag/v2.4.0`, subject_path: null, reason: "subscribed", unread: false, local_unread: false, updated_at: iso(60 * 30), state: null, participants: ["dev-alex"], participant_count: 1, review_decision: null, checks: null },
  ].sort((a, b) => b.updated_at.localeCompare(a.updated_at));
  const predrafts: Predraft[] = [
    predraft({
      id: "demo-pr-refunds", project_id: P.docs, root: WT.refunds, branch: "lp/document-refund-flow", initial_base: DEMO_COMMITS.docsHead, session_id: "demo-s-docs", created_at: clock.ago(170),
      title: "Document the refund flow", body: "Adds `guide/refunds.md`: full and partial refunds, and the refund states.\n\n- New page in the navigation\n- A table of the refund states",
      notes: [{ path: "guide/refunds.md", text: "Line 14: Say whether the customer also gets an email for a partial refund." }],
    }),
    predraft({
      id: "demo-pr-prices", project_id: P.storefront, root: WT.prices, branch: "lp/format-prices", initial_base: DEMO_COMMITS.storefrontPrev, created_at: clock.ago(60 * 24),
      title: "Format prices in the visitor's locale", body: "`formatMoney` now uses the browser's locale by default instead of `en-US`.",
      notes: [{ path: null, text: "Keep `en-US` for server-side rendering, where there is no navigator." }],
      merged: { checkout: `${ROOT}/storefront`, head: DEMO_COMMITS.pricesMerged, at: clock.ago(60 * 20) },
    }),
  ];
  const suggestions: Suggestion[] = [{
    id: "demo-sg-118", target: { kind: "git_hub", repo: REPO.storefront, number: 118 }, kind: "review", event: "COMMENT", head: it(REPO.storefront, 118).headRefOid ?? null,
    author: "Claude Code", session_id: null, created_at: clock.ago(40), status: "draft", url: null,
    body: "The saved carts work well and the tests cover the limit of five.\n\nTwo suggestions before merging:\n\n- `savedCarts()` throws when the stored value is not valid JSON; return `[]` instead.\n- `saveCart` stores the full cart lines, including prices. Prices can change, so store the SKUs and quantities only and read prices again when the cart is restored.",
  }];
  /** Review state values (`/api/review-state`): viewed markers, unsent replies and drafts. */
  const reviewState: Record<string, { value: unknown; updated_at: number }> = {
    [`viewed:${REPO.storefront}#118@${it(REPO.storefront, 118).headRefOid}`]: { value: ["tests/savedCarts.test.ts"], updated_at: clock.ago(30) },
    [`reply:${REPO.storefront}#119`]: { value: "Looks good. Can the placeholder keep a fixed height, so the page does not jump?", updated_at: clock.ago(20) },
  };
  return { ghItems, inboxThreads, doneThreads: [] as Thread[], predrafts, suggestions, reviewState, prDiffs: {} as Record<string, Change[]> };
}

/** The changes of a demo PR (built on first use from the folder trees). */
export function prDiff(repo: string, number: number): Change[] {
  const key = `${repo}#${number}`;
  if (!world.prDiffs[key]) world.prDiffs[key] = prChanges()[key] ?? [];
  return world.prDiffs[key];
}

/** Copies the current state of an item into its inbox threads (after a simulated GitHub action). */
export function syncThreads(item: Item) {
  for (const t of world.inboxThreads) {
    if (t.repo !== item.repo || t.number !== item.number) continue;
    t.state = threadState(item);
    t.title = item.title;
    t.review_decision = item.reviewDecision ?? null;
    t.updated_at = item.updatedAt;
  }
  publish("inbox", "updated", {});
}

// ---------------------------------------------------------------- scenario player API

/**
 * Adds a Local PR (pre-draft) with its worktree folder and publishes `predrafts/updated`. `changes` maps paths
 * of the folder to their new content (`null` deletes a file); Changes, Files and Review read them.
 */
export function addLocalPr(o: { id?: string; projectId: string; title: string; body: string; sessionId: string | null; root: string; branch: string; changes: Record<string, string | null>; notes?: Note[] }): Predraft {
  const base = o.projectId === P.storefront ? DEMO_COMMITS.storefrontHead : o.projectId === P.billing ? DEMO_COMMITS.billingHead : DEMO_COMMITS.docsHead;
  world.codeRoots[o.root] = { projectId: o.projectId, branch: o.branch, base: "main", overrides: { ...o.changes }, exists: true };
  const p = predraft({ id: o.id ?? `demo-pr-${clock.now().toString(36)}`, project_id: o.projectId, root: o.root, branch: o.branch, title: o.title, body: o.body, session_id: o.sessionId, initial_base: base, notes: o.notes ?? [], created_at: clock.now() });
  world.predrafts = [p, ...world.predrafts.filter((x) => x.id !== p.id)];
  publish("predrafts", "updated", p);
  return p;
}

/** Adds an agent-written comment or review suggestion on a GitHub PR or issue and publishes `suggestions/updated`. */
export function addSuggestion(o: { repo: string; number: number; kind: SuggestionKind; body: string; event?: string; sessionId: string | null; author?: string }): Suggestion {
  const item = world.ghItems[`${o.repo}#${o.number}`];
  const s: Suggestion = {
    id: `demo-sg-${clock.now().toString(36)}`, target: { kind: "git_hub", repo: o.repo, number: o.number }, kind: o.kind, body: o.body, event: o.event ?? "COMMENT",
    head: o.kind === "review" ? item?.headRefOid ?? null : null, author: o.author ?? "OpenCode", session_id: o.sessionId, created_at: clock.now(), status: "draft", url: null,
  };
  world.suggestions = [...world.suggestions, s];
  publish("suggestions", "updated", s);
  return s;
}

/** Optional hook of the scenario player: called when the owner sends Local PR notes to its agent. */
export const localPrHooks: { feedback: ((p: Predraft, text: string) => void) | null } = { feedback: null };

/** Folder of the Local PR of the issue #42 scenario; the scenario's session should use it as `cwd`. */
export const ISSUE42_ROOT = `${ROOT}/.worktrees/${P.docs}/explain-webhook-retries`;

/**
 * The end of the "Draft a Local PR for issue #42" scenario (docs, OpenCode): a Local PR that documents webhook
 * retries, and a review suggestion on docs PR #44. Call it once when the run finishes.
 */
export function finishIssue42(sessionId: string | null): { predraft: Predraft; suggestion: Suggestion } {
  const page = world.codeTrees[P.docs]["api/webhooks.md"];
  const predraft = addLocalPr({
    id: "demo-pr-webhooks", projectId: P.docs, sessionId, root: ISSUE42_ROOT, branch: "lp/explain-webhook-retries",
    title: "Explain webhook retries (#42)", body: "Documents how billing retries failed webhooks.\n\n- Five attempts, 1 s to 16 s apart\n- What counts as a failure\n- How to make an endpoint idempotent\n\nCloses #42",
    changes: { "api/webhooks.md": `${page}
## Retries

A delivery fails when your endpoint does not answer with a 2xx status within 10 seconds.
Billing then retries up to five times, waiting 1, 2, 4, 8 and 16 seconds.

## Idempotent endpoints

A retry can deliver the same event twice. Store the event \`id\` and ignore events you already handled.
` },
  });
  const suggestion = addSuggestion({ repo: REPO.docs, number: 44, kind: "review", event: "APPROVE", sessionId, body: "The new link is in the right place and the page builds. While you are there, the refunds page now exists (Local PR \"Document the refund flow\"), so this link will work once both are merged." });
  return { predraft, suggestion };
}
