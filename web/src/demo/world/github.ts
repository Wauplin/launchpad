// GitHub part of the demo world (Local PRs, suggestions, review state, and the few issues and pull requests the
// composer's `#` suggestions name). Never contacted: fake data only. Launchpad does not show remote items: links open
// github.com.
//
// For the scenario player: `addLocalPr`, `addSuggestion` and `finishIssue42` (below) add a Local PR and an
// agent-written suggestion at the end of the "Draft a Local PR for issue #42" run and publish the bus events,
// so Today, the folder page and the Local PR page update like with the real daemon.

import type { GhItem } from "@/gen/GhItem";
import type { Note } from "@/gen/Note";
import type { Predraft } from "@/gen/Predraft";
import type { Suggestion } from "@/gen/Suggestion";
import type { SuggestionKind } from "@/gen/SuggestionKind";
import { clock, publish } from "../daemon";
import { world } from "./index";
import { DEMO_COMMITS, WT } from "./code";
import { ORG, P, ROOT } from "./ids";

export const REPO = { storefront: `${ORG}/storefront`, billing: `${ORG}/billing-api`, docs: `${ORG}/docs` } as const;

// ---------------------------------------------------------------- helpers

const iso = (minutesAgo: number) => new Date(clock.ago(minutesAgo)).toISOString();

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

// ---------------------------------------------------------------- issues and pull requests

/** What GitHub's search would answer for the composer's `#` suggestions (`/api/gh/{repo}/suggest`). */
function ghItems(): GhItem[] {
  const specs: [string, number, "pr" | "issue", string, string, string, number][] = [
    [REPO.storefront, 118, "pr", "Save carts for later", "open", "dev-alex", 35],
    [REPO.storefront, 119, "pr", "Lazy-load product images", "open", "dev-rui", 50],
    [REPO.storefront, 121, "pr", "Upgrade the test runner to vitest 3", "open", "dev-kim", 95],
    [REPO.billing, 57, "pr", "Retry webhook deliveries", "draft", "dev-sam", 60 * 4],
    [REPO.billing, 55, "pr", "Add invoice PDF export", "merged", "dev-sam", 60 * 6],
    [REPO.docs, 44, "pr", "Link refunds from the checkout guide", "open", "dev-kim", 70],
    [REPO.docs, 42, "issue", "Explain webhook retries", "open", "dev-sam", 60 * 2],
    [REPO.storefront, 120, "issue", "Checkout test is flaky on CI", "open", "dev-rui", 45],
    [REPO.storefront, 115, "issue", "Support gift cards at checkout", "open", "dev-kim", 60 * 7],
    [REPO.billing, 58, "issue", "Rate-limit /invoices", "open", "dev-rui", 25],
    [REPO.billing, 50, "issue", "Invoice totals round the wrong way", "closed", "demo-user", 60 * 24],
    [REPO.docs, 40, "issue", "Add a glossary page", "open", "dev-alex", 60 * 24 * 3],
  ];
  return specs.map(([repo, number, kind, title, state, author, ago]) => ({ repo, number, kind, title, state, author, url: `https://github.com/${repo}/${kind === "pr" ? "pull" : "issues"}/${number}`, updated_at: iso(ago) }));
}

// ---------------------------------------------------------------- Local PRs and suggestions

/** Demo only: the worktree folder that shows each Local PR's changes (real Local PRs live on Home Git). */
const predraftRoots: Record<string, string> = {};
export const predraftRoot = (p: Predraft) => predraftRoots[p.id] ?? "";
const predraft = ({ root, ...p }: Partial<Predraft> & Pick<Predraft, "id" | "project_id" | "branch" | "title" | "body"> & { root: string }): Predraft => {
  predraftRoots[p.id] = root;
  return {
    base_branch: "main", initial_base: "", session_id: null, notes: [], archived: false, published_url: null, published_number: null,
    default_remote_branch: p.branch.replace(/^lp\//, ""), remote_branch: null, pushed_head: null, created_at: clock.ago(60), merged: null, change_set: p.session_id ?? null, ...p,
  };
};

export function githubWorld() {
  const predrafts: Predraft[] = [
    predraft({
      id: "demo-pr-refunds", project_id: P.docs, root: WT.refunds, branch: "lp/document-refund-flow", initial_base: DEMO_COMMITS.docsHead, session_id: "demo-s-docs", created_at: clock.ago(170),
      title: "Document the refund flow", body: "Adds `guide/refunds.md`: full and partial refunds, and the refund states.\n\n- New page in the navigation\n- A table of the refund states",
      notes: [{ path: "guide/refunds.md", line: 14, side: "new", text: "Say whether the customer also gets an email for a partial refund." }],
    }),
    predraft({
      id: "demo-pr-prices", project_id: P.storefront, root: WT.prices, branch: "lp/format-prices", initial_base: DEMO_COMMITS.storefrontPrev, created_at: clock.ago(60 * 24),
      title: "Format prices in the visitor's locale", body: "`formatMoney` now uses the browser's locale by default instead of `en-US`.",
      notes: [{ path: null, text: "Keep `en-US` for server-side rendering, where there is no navigator." }],
      merged: { head: DEMO_COMMITS.pricesMerged, at: clock.ago(60 * 20) },
    }),
  ];
  const suggestions: Suggestion[] = [{
    id: "demo-sg-118", target: { kind: "git_hub", repo: REPO.storefront, number: 118 }, kind: "review", event: "COMMENT", head: fakeSha(`${REPO.storefront}#118:head`),
    author: "Claude Code", session_id: null, created_at: clock.ago(40), status: "draft", url: null, merge: null, run_ci: false, followup_error: null,
    body: "The saved carts work well and the tests cover the limit of five.\n\nTwo suggestions before merging, as line comments: a corrupted stored value breaks the cart, and saved prices go stale.",
    comments: [
      { path: "src/savedCarts.ts", line: 7, side: "RIGHT", body: "`JSON.parse` throws when the stored value is not valid JSON. Return `[]` instead:\n\n```suggestion\n  try { return JSON.parse(localStorage.getItem(KEY) ?? \"[]\") as Cart[]; } catch { return []; }\n```" },
      { path: "src/savedCarts.ts", line: 12, side: "RIGHT", body: "This stores the full cart lines, including prices. Prices can change: store the SKUs and quantities only and read prices again when the cart is restored." },
    ],
  }];
  /** Review state values (`/api/review-state`): viewed markers and drafts. */
  const reviewState: Record<string, { value: unknown; updated_at: number }> = {};
  return { ghItems: ghItems(), predrafts, suggestions, reviewState };
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
  const s: Suggestion = {
    id: `demo-sg-${clock.now().toString(36)}`, target: { kind: "git_hub", repo: o.repo, number: o.number }, kind: o.kind, body: o.body, event: o.event ?? "COMMENT",
    head: o.kind === "review" ? fakeSha(`${o.repo}#${o.number}:head`) : null, author: o.author ?? "OpenCode", session_id: o.sessionId, created_at: clock.now(), status: "draft", url: null, merge: null, run_ci: false, followup_error: null, comments: [],
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
