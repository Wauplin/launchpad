// Every place the GitHub layer inserts something, in this ONE file: a named anchor with two or three selector
// strategies, most stable first (ids and data attributes, then ARIA roles and labels, then visible text). When GitHub
// changes its DOM, the anchor stops matching, its feature shows nothing on the page (the side panel covers it) and
// Settings says which anchors work. Strategies only locate elements: IDs and numbers always come from the URL and
// Home, never from page text. Nothing here clicks or submits anything.

import type { ExtPageKind } from "@/gen/ExtPageKind";
import { request } from "../../shared/messages";

export type Params = { repo: string; number: number | null };
type Via = "id" | "data" | "aria" | "text";
/** `place` overrides the anchor's own, for a strategy that finds another kind of element. */
type Strategy = { via: Via; find: (p: Params) => Element | null; place?: InsertPosition };
type Anchor = {
  /** Its name in Settings. */
  label: string;
  /** The page kinds where it must exist. */
  kinds: readonly ExtPageKind[];
  /** Where a widget goes, relative to the element found. */
  place: InsertPosition;
  strategies: readonly Strategy[];
};

const REPO_KINDS: readonly ExtPageKind[] = ["repo", "pulls", "issues", "pr_conversation", "pr_files", "pr_commits", "issue", "actions_run", "compare", "new_release"];

const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();
/** The path of a github.com link (relative or absolute), "" for any other. */
const pathOf = (a: Element | null) => {
  try {
    const u = new URL(a?.getAttribute("href") ?? "", location.origin);
    return u.origin === location.origin ? u.pathname.replace(/\/$/, "") : "";
  } catch { return ""; }
};
/** A link to the repository itself (`/owner/name`). */
const linksRepo = (a: Element | null, p: Params) => same(pathOf(a), `/${p.repo}`);
const all = (selector: string, root: ParentNode = document) => Array.from(root.querySelectorAll(selector));

export const ANCHORS = {
  /** The repository's name in the page header (repository badge, R1). */
  "repo-name": {
    label: "Repository name",
    kinds: REPO_KINDS,
    place: "afterend",
    strategies: [
      // Repository home: the title component, after its visibility label ("Public").
      { via: "data", find: (p) => {
        const wrapper = all('[data-testid="repo-name-wrapper"]').find((w) => linksRepo(w.querySelector('a[data-testid="repo-name-link"], a[href]'), p));
        if (!wrapper) return null;
        const label = wrapper.parentElement?.querySelector(':scope > [data-testid="repo-visibility-label"]');
        return label ?? wrapper;
      } },
      // Older layouts: the repository header's name.
      { via: "id", find: (p) => all('#repository-container-header strong[itemprop="name"]').find((s) => linksRepo(s.querySelector("a"), p)) ?? null },
      // Every page of a repository: the global header's breadcrumbs, ending with the repository (after its link).
      { via: "aria", find: (p) => {
        for (const nav of all('nav[aria-label="Breadcrumbs"], nav[aria-label="Page context"]')) {
          const links = nav.querySelectorAll("a[href]");
          const last = links[links.length - 1] ?? null;
          if (linksRepo(last, p)) return last;
        }
        return null;
      } },
      { via: "text", find: (p) => all("header a[href]").find((a) => linksRepo(a, p) && same(a.textContent?.trim() ?? "", p.repo.split("/")[1])) ?? null },
    ],
  },
  /** The repository's tabs (Code, Issues, Pull requests…): the "+2 local" count (L1). */
  "repo-tabs": {
    label: "Repository tabs",
    kinds: REPO_KINDS,
    place: "beforeend",
    strategies: [
      { via: "id", find: () => document.querySelector("nav.js-repo-nav") },
      { via: "aria", find: () => document.querySelector('nav[aria-label="Repository"]') },
      { via: "text", find: (p) => all("nav").find((n) => all("a[href]", n).some((a) => same(pathOf(a), `/${p.repo}/pulls`) && /pull requests/i.test(a.textContent ?? ""))) ?? null },
    ],
  },
  /** The rows of a Pull requests or Issues list (row chips, L2). */
  "list-rows": {
    label: "Pull request and issue rows",
    kinds: ["pulls", "issues"],
    place: "afterend",
    strategies: [
      { via: "data", find: () => document.querySelector('[data-testid="listitem-title-link"], [data-testid="issue-listitem-title-link"]') },
      { via: "id", find: () => document.querySelector(".js-issue-row a.Link--primary") },
      { via: "aria", find: (p) => all('[role="list"] a[href], [role="grid"] a[href], ul a[href]').find((a) => {
        const path = pathOf(a).toLowerCase();
        return path.startsWith(`/${p.repo.toLowerCase()}/`) && /^\/[^/]+\/[^/]+\/(pull|issues)\/\d+$/.test(path);
      }) ?? null },
    ],
  },
  /** The title of an issue or pull request (live agent cards, I5). */
  "item-title": {
    label: "Issue and pull request title",
    kinds: ["pr_conversation", "pr_files", "pr_commits", "issue"],
    place: "afterend",
    strategies: [
      { via: "data", find: () => document.querySelector('[data-testid="issue-title"], [data-component="PH_Title"]') },
      { via: "id", find: () => document.querySelector(".gh-header-title, #partial-discussion-header h1") },
      { via: "aria", find: (p) => all("main h1, h1").find((h) => h.textContent?.includes(`#${p.number}`)) ?? null },
    ],
  },
} satisfies Record<string, Anchor>;

export type AnchorName = keyof typeof ANCHORS;

/** One observed root per page kind for the scoped MutationObserver: the page body without the document itself. */
export const ROOTS = ["div[data-turbo-body]", "body > div.logged-in", "body > div.logged-out", "body > div"] as const;

// Health: each anchor expected on the page is checked; the result of the last check of a page is reported to the
// worker once the page settled (React renders after the HTML), so Settings can say "11 of 12 work".

type Found = { el: Element; place: InsertPosition; via: Via };
let page: { kind: ExtPageKind; params: Params | null; results: Map<AnchorName, Via | null> } = { kind: "other", params: null, results: new Map() };
let flushTimer: ReturnType<typeof setTimeout> | undefined;
const SETTLE_MS = 4_000;

/** Finds an anchor on the current page and records the result. */
export function findAnchor(name: AnchorName, p: Params): Found | null {
  const anchor: Anchor = ANCHORS[name];
  for (const s of anchor.strategies) {
    let el: Element | null = null;
    try { el = s.find(p); } catch { el = null; }
    if (el && el.isConnected) {
      page.results.set(name, s.via);
      return { el, place: s.place ?? anchor.place, via: s.via };
    }
  }
  if (!page.results.get(name)) page.results.set(name, null);
  return null;
}

/** A new page: checks start again and are reported once it settled. */
export function beginPage(kind: ExtPageKind, params: Params | null) {
  clearTimeout(flushTimer);
  page = { kind, params, results: new Map() };
  if (!params) return;
  flushTimer = setTimeout(() => void flush(), SETTLE_MS);
}

async function flush() {
  const { kind, params, results } = page;
  if (!params) return;
  const checks: Record<string, { label: string; ok: boolean; strategy: string | null; kind: string }> = {};
  for (const [name, anchor] of Object.entries(ANCHORS) as [AnchorName, Anchor][]) {
    if (!anchor.kinds.includes(kind)) continue;
    // Anchors no feature looked for on this page are checked now.
    const via = results.get(name) ?? findAnchor(name, params)?.via ?? null;
    checks[name] = { label: anchor.label, ok: via !== null, strategy: via, kind };
  }
  if (Object.keys(checks).length) await request("anchors.report", { checks }).catch(() => undefined);
}
