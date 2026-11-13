// Every place the GitHub layer inserts something, in this ONE file: a named anchor with two or three selector
// strategies, most stable first (ids and data attributes, then ARIA roles and labels, then visible text). When GitHub
// changes its DOM, the anchor stops matching, its feature shows nothing on the page (the side panel covers it) and
// Settings says which anchors work. Strategies only locate elements: IDs and numbers always come from the URL and
// Home, never from page text. Nothing here clicks or submits anything.

import type { ExtPageKind } from "@/gen/ExtPageKind";
import { request } from "../../shared/messages";

export type Params = {
  repo: string;
  number: number | null;
  /** A comment's anchor (`issuecomment-1`, `discussion_r2`). */
  comment?: string;
  /** A line of a pull request's diff: its file, side and number (Files changed). */
  path?: string;
  side?: "LEFT" | "RIGHT";
  line?: number;
};
type Via = "id" | "data" | "aria" | "text";
/** `place` overrides the anchor's own, for a strategy that finds another kind of element. */
type Strategy = { via: Via; find: (p: Params) => Element | null; place?: InsertPosition; /** Lists: every match. */ all?: (p: Params) => Element[] };
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
  /** Each row of a Pull requests or Issues list (row chips L2, hover review L3, selection L4). */
  "list-items": {
    label: "Pull request and issue list rows",
    kinds: ["pulls", "issues"],
    place: "beforeend",
    strategies: [
      { via: "data", find: () => document.querySelector('[data-listview-component="items-list"] > li'), all: () => all('[data-listview-component="items-list"] > li') },
      { via: "id", find: () => document.querySelector(".js-issue-row"), all: () => all(".js-issue-row") },
      { via: "aria", find: (p) => listRows(p)[0] ?? null, all: (p) => listRows(p) },
    ],
  },
  /** The list of a Pull requests page: the Local PRs section goes above it (L1). */
  "list-view": {
    label: "Pull request list",
    kinds: ["pulls"],
    place: "beforebegin",
    strategies: [
      { via: "data", find: () => document.querySelector('[data-listview-component="items-list"]')?.closest('[id$="list-view-container"]') ?? document.querySelector('[data-listview-component="items-list"]') },
      { via: "id", find: () => document.querySelector("#js-issues-toolbar")?.closest(".Box") ?? null },
      { via: "aria", find: (p) => listRows(p)[0]?.closest("ul, ol, [role=list]") ?? null },
    ],
  },
  /** The Pull requests tab of the repository: "+2 local" after its count (L1). */
  "pr-tab": {
    label: "Pull requests tab",
    kinds: REPO_KINDS,
    place: "afterend",
    strategies: [
      { via: "data", find: () => document.querySelector('nav[aria-label="Repository"] a[data-tab-item="pull-requests"], a[data-tab-item="i2pull-requests-tab"]') },
      { via: "id", find: () => document.querySelector("a#pull-requests-tab") },
      { via: "text", find: (p) => all("nav a[href]").find((a) => same(pathOf(a), `/${p.repo}/pulls`) && /pull requests/i.test(a.textContent ?? "")) ?? null },
    ],
  },
  /** The top of a repository's content, under its tabs: the Launchpad strip (R2). */
  "repo-content": {
    label: "Repository content",
    kinds: ["repo"],
    place: "afterbegin",
    strategies: [
      { via: "id", find: () => document.querySelector("#repo-content-pjax-container") ?? document.querySelector("#repo-content-turbo-frame") },
      { via: "aria", find: () => document.querySelector('main[id="js-repo-pjax-container"], main') },
    ],
  },
  /** The list of github.com/notifications: the Today strip goes above it (N1). */
  "notification-list": {
    label: "Notifications list",
    kinds: ["notifications"],
    place: "beforebegin",
    strategies: [
      { via: "id", find: () => document.querySelector(".js-notifications-container") },
      { via: "data", find: () => document.querySelector("[data-notification-id]")?.closest(".Box, ul") ?? null },
      { via: "aria", find: () => notificationRows()[0]?.closest("ul, ol, [role=list]") ?? null },
    ],
  },
  /** Each notification row (row status N2, observed Done N3, hover Ask N5). */
  "notification-rows": {
    label: "Notification rows",
    kinds: ["notifications"],
    place: "beforeend",
    strategies: [
      { via: "data", find: () => document.querySelector("li[data-notification-id]"), all: () => all("li[data-notification-id]") },
      { via: "id", find: () => document.querySelector(".notifications-list-item"), all: () => all(".notifications-list-item") },
      { via: "aria", find: () => notificationRows()[0] ?? null, all: () => notificationRows() },
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
  /** The first block of a pull request's or issue's sidebar: the Launchpad box (I1). */
  "sidebar": {
    label: "Sidebar of a pull request or issue",
    kinds: ["pr_conversation", "issue"],
    place: "afterbegin",
    strategies: [
      { via: "id", find: () => document.querySelector("#partial-discussion-sidebar") },
      { via: "data", find: () => document.querySelector('[data-testid="issue-viewer-metadata-pane"] [data-testid="sticky-sidebar"], [data-testid="issue-viewer-metadata-pane"]') },
      { via: "aria", find: () => all("main h2").find((h) => /^metadata$/i.test(h.textContent?.trim() ?? ""))?.parentElement ?? null },
    ],
  },
  /** The header actions of each comment (Ask a bot, I2): its element carries the comment's anchor as `id` or inside. */
  "comments": {
    label: "Comment headers",
    kinds: ["pr_conversation", "issue"],
    place: "afterbegin",
    strategies: [
      // Classic conversation (pull requests): the comment group, with its header's actions.
      { via: "id", find: () => document.querySelector('.timeline-comment-group[id^="issuecomment-"] .timeline-comment-actions, .review-comment[id^="discussion_r"] .timeline-comment-actions'),
        all: () => all('.timeline-comment-group[id^="issuecomment-"], .review-comment[id^="discussion_r"]').map((g) => g.querySelector(".timeline-comment-actions")).filter((x): x is Element => !!x) },
      // React issues: the comment header (id issuecomment-…) and its right-side items.
      { via: "data", find: () => document.querySelector('[data-testid="comment-header"][id^="issuecomment-"] [data-testid="comment-header-right-side-items"]'),
        all: () => all('[data-testid="comment-header"][id^="issuecomment-"] [data-testid="comment-header-right-side-items"]') },
    ],
  },
  /** The conversation's comment box at the bottom (Insert, I3). Absent when signed out of GitHub. */
  "comment-box": {
    label: "Comment box",
    kinds: [],
    place: "beforebegin",
    strategies: [
      { via: "id", find: () => document.querySelector("textarea#new_comment_field") },
      { via: "data", find: () => all('[data-testid="issue-viewer-comments-container"] ~ * textarea, [data-testid="markdown-editor-comment-composer"] textarea, form[aria-label*="comment" i] textarea').at(-1) ?? null },
      { via: "aria", find: () => all('main textarea[placeholder*="comment" i], main textarea[aria-label*="comment" i], main textarea[aria-label="Markdown value"]').filter((t) => !t.closest(".js-inline-comment-form-container, .review-thread-reply, [data-launchpad]")).at(-1) ?? null },
    ],
  },
  /** The reply box of the review thread of comment `p.comment` (`discussion_r…`), once GitHub opened it (Insert of a thread reply). */
  "thread-reply": {
    label: "Review thread reply box",
    kinds: [],
    place: "beforebegin",
    strategies: [
      { via: "id", find: (p) => {
        const c = p.comment ? document.getElementById(p.comment) : null;
        const thread = c?.closest(".js-resolvable-timeline-thread-container, .review-thread-component, .js-inline-comments-container, .inline-comments");
        return thread ? all('textarea[name="comment[body]"], textarea', thread).find((t) => t.getClientRects().length > 0) ?? null : null;
      } },
      { via: "aria", find: (p) => {
        const c = p.comment ? document.getElementById(p.comment) : null;
        const thread = c?.closest('[role="group"], section, li') ?? null;
        return thread ? all("textarea", thread).find((t) => t.getClientRects().length > 0) ?? null : null;
      } },
    ],
  },
  /** GitHub's own "Reply…" field of a review thread (focused, never clicked, so the owner opens it). */
  "thread-reply-opener": {
    label: "Review thread Reply field",
    kinds: [],
    place: "beforebegin",
    strategies: [
      { via: "id", find: (p) => {
        const c = p.comment ? document.getElementById(p.comment) : null;
        const thread = c?.closest(".js-resolvable-timeline-thread-container, .review-thread-component, .js-inline-comments-container, .inline-comments");
        return thread?.querySelector(".review-thread-reply-button, button.js-toggle-inline-comment-form") ?? null;
      } },
      { via: "text", find: (p) => {
        const c = p.comment ? document.getElementById(p.comment) : null;
        const thread = c?.closest(".js-resolvable-timeline-thread-container, .review-thread-component, [role=\"group\"]");
        return thread ? all("button", thread).find((b) => /^reply/i.test(b.textContent?.trim() ?? "")) ?? null : null;
      } },
    ],
  },
  /** The files of a pull request's Files changed, each with its path (Ask about these lines D1, ghost comments D2). */
  "diff-files": {
    label: "Files changed: files",
    kinds: ["pr_files"],
    place: "afterend",
    strategies: [
      { via: "data", find: () => document.querySelector("[data-tagsearch-path]"), all: () => all("[data-tagsearch-path]") },
      { via: "id", find: () => document.querySelector('copilot-diff-entry[data-file-path] [id^="diff-"]'), all: () => all('copilot-diff-entry[data-file-path] [id^="diff-"]') },
      { via: "aria", find: () => document.querySelector('[data-file-path][id^="diff-"], [data-file-path] [id^="diff-"]'), all: () => all('[data-file-path][id^="diff-"], [data-file-path] [id^="diff-"]') },
    ],
  },
  /** One line of a pull request's diff (`p.path`, `p.side`, `p.line`; without them, any line for the health check): its row,
   * where an answer or a ghost comment goes. GitHub's line anchors are `diff-<sha256 of the path><L|R><line>`. */
  "diff-line": {
    label: "Files changed: diff lines",
    kinds: ["pr_files"],
    place: "afterend",
    strategies: [
      { via: "id", find: (p) => {
        if (!p.path) return document.querySelector('td.blob-num[id^="diff-"][data-line-number]')?.closest("tr") ?? null;
        const file = fileOf(p.path);
        const cell = file ? document.getElementById(`${file.id}${p.side === "LEFT" ? "L" : "R"}${p.line}`) : null;
        return cell?.closest("tr") ?? null;
      } },
      { via: "data", find: (p) => {
        if (!p.path) return document.querySelector('[data-line-number][data-diff-anchor], [data-grid-cell-id]')?.closest('tr, [role="row"]') ?? null;
        const hash = hashes.get(p.path);
        const id = hash ? `diff-${hash}${p.side === "LEFT" ? "L" : "R"}${p.line}` : null;
        const cell = id ? document.getElementById(id) ?? document.querySelector(`[data-line-anchor="${id}"], [data-grid-cell-id^="${id}"]`) : null;
        return cell?.closest('tr, [role="row"]') ?? null;
      } },
    ],
  },
  /** GitHub's own line comment form of a diff line, once the owner opened it (Add to my review, Turn into line comment). */
  "line-form": {
    label: "Line comment form",
    kinds: [],
    place: "beforebegin",
    strategies: [
      // Classic: the inline comment row GitHub adds right after the line's row.
      { via: "id", find: (p) => {
        const row = p.path ? findLine(p) : null;
        for (let r = row?.nextElementSibling ?? null, i = 0; r && i < 3; r = r.nextElementSibling, i++) {
          if (r.hasAttribute("data-launchpad-row")) continue;
          if (!r.matches(".inline-comments, .js-inline-comments-container")) break;
          const t = visibleTextarea(r);
          if (t) return t;
        }
        return null;
      } },
      // The form's own fields name the line.
      { via: "data", find: (p) => all("form").find((f) => (f.querySelector('input[name="path"]') as HTMLInputElement | null)?.value === p.path
        && (f.querySelector('input[name="line"]') as HTMLInputElement | null)?.value === String(p.line)
        && ((f.querySelector('input[name="side"]') as HTMLInputElement | null)?.value ?? "RIGHT").toUpperCase() === p.side
        && !!visibleTextarea(f))?.querySelector("textarea") ?? null },
      // Newer layouts: the text field GitHub focused in this file's diff after the owner opened a line's form, when it
      // sits after this line and before the next one.
      { via: "aria", find: (p) => {
        const t = lastFocused && Date.now() - lastFocused.at < 120_000 && lastFocused.path === p.path ? lastFocused.box : null;
        const row = p.path ? findLine(p) : null;
        if (!t?.isConnected || !row || t.getClientRects().length === 0 || !(row.compareDocumentPosition(t) & Node.DOCUMENT_POSITION_FOLLOWING)) return null;
        const next = all("[data-line-number]").find((el) => !row.contains(el) && !!(row.compareDocumentPosition(el) & Node.DOCUMENT_POSITION_FOLLOWING));
        return !next || !!(t.compareDocumentPosition(next) & Node.DOCUMENT_POSITION_FOLLOWING) ? t : null;
      } },
    ],
  },
  /** The summary field of GitHub's Finish your review / Review changes dialog (Insert summary, D3). */
  "review-dialog": {
    label: "Review changes dialog",
    kinds: [],
    place: "beforebegin",
    strategies: [
      { via: "id", find: () => visible(document.querySelector("textarea#pull_request_review_body")) },
      { via: "data", find: () => all('form[action$="/reviews"] textarea, .js-reviews-container textarea').map(visible).find(Boolean) ?? null },
      { via: "aria", find: () => all('[role="dialog"]').filter((d) => /finish your review|review changes|submit review/i.test(d.querySelector("h1, h2, h3, [id$='title'], [class*='Title']")?.textContent ?? d.getAttribute("aria-label") ?? "")).map((d) => visibleTextarea(d)).find(Boolean) ?? null },
    ],
  },
  /** GitHub's "Approve and run" of workflows waiting for a maintainer on a fork's pull request (CI note, I8). */
  "ci-approval": {
    label: "Approve and run workflows",
    kinds: [],
    place: "afterend",
    strategies: [
      { via: "data", find: () => all('form[action*="approve"] button, form[action*="approve"] input[type="submit"]').find((b) => /approve and run/i.test(b.textContent ?? (b as HTMLInputElement).value ?? ""))?.closest(".branch-action-item, .merge-pr, .mergeability-details, section, .Box, form") ?? null },
      { via: "text", find: () => all("main button").find((b) => /^approve and run/i.test(b.textContent?.trim() ?? ""))?.closest(".branch-action-item, section, [class*='MergeBox'], .Box, div") ?? null },
    ],
  },
} satisfies Record<string, Anchor>;

/** sha256 of each diff path seen (GitHub's line anchors use it), filled by `hashPaths`. */
const hashes = new Map<string, string>();
export async function hashPaths(paths: string[]) {
  for (const p of paths) {
    if (hashes.has(p)) continue;
    const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(p));
    hashes.set(p, Array.from(new Uint8Array(digest), (b) => b.toString(16).padStart(2, "0")).join(""));
  }
}

/** The diff container of a file of Files changed (its id is `diff-<sha256 of the path>`). */
export function fileOf(path: string): Element | null {
  const byData = all("[data-tagsearch-path]").find((f) => f.getAttribute("data-tagsearch-path") === path);
  if (byData?.id.startsWith("diff-")) return byData;
  const entry = all("copilot-diff-entry[data-file-path], [data-file-path]").find((f) => f.getAttribute("data-file-path") === path);
  const inner = entry?.id.startsWith("diff-") ? entry : entry?.querySelector('[id^="diff-"]');
  if (inner) return inner;
  const hash = hashes.get(path);
  return hash ? document.getElementById(`diff-${hash}`) : null;
}

/** The path of a diff file from its `diff-<hash>` anchor (a line selection's URL fragment). */
export function pathOfHash(hash: string): string | null {
  const el = document.getElementById(`diff-${hash}`);
  const path = el?.getAttribute("data-tagsearch-path") ?? el?.closest("[data-file-path]")?.getAttribute("data-file-path") ?? null;
  if (path) return path;
  for (const [p, h] of hashes) if (h === hash) return p;
  return null;
}

/** The row of a diff line, or null. */
export function findLine(p: Params): Element | null {
  return findAnchor("diff-line", p)?.el ?? null;
}

function visible(el: Element | null): HTMLTextAreaElement | null {
  return el instanceof HTMLTextAreaElement && el.getClientRects().length > 0 ? el : null;
}
function visibleTextarea(root: Element): HTMLTextAreaElement | null {
  return all("textarea", root).map(visible).find((t) => t && !t.closest("[data-launchpad]")) ?? null;
}

/** The last GitHub text field focused inside a diff file (newer layouts open a line's form there and focus it). */
let lastFocused: { box: HTMLTextAreaElement; path: string; at: number } | null = null;
document.addEventListener("focusin", (e) => {
  const t = e.target;
  if (!(t instanceof HTMLTextAreaElement)) return;
  const file = t.closest("[data-tagsearch-path], [data-file-path]");
  const path = file?.getAttribute("data-tagsearch-path") ?? file?.getAttribute("data-file-path");
  if (path) lastFocused = { box: t, path, at: Date.now() };
}, true);

/** Rows of a list found by their links to the repository's pull requests or issues. */
function listRows(p: Params): Element[] {
  const rows = all("main li").filter((li) => all("a[href]", li).some((a) => new RegExp(`^/${p.repo.replace(/[.]/g, "\\.")}/(pull|issues)/\\d+$`, "i").test(pathOf(a))));
  return rows.filter((r) => !rows.some((o) => o !== r && r.contains(o)));
}

/** Rows of the notifications list found by their links (GitHub adds `notification_referrer_id`). */
function notificationRows(): Element[] {
  return all('a[href*="notification_referrer_id"]').map((a) => a.closest("li")).filter((x): x is HTMLLIElement => !!x);
}

/** The item a list or notification row links to: its repository and number, from the row's link (never its text). */
export function rowItem(row: Element): { repo: string; number: number; kind: "pr" | "issue"; url: string; title: string | null } | null {
  for (const a of all('a[href]', row)) {
    const m = pathOf(a).match(/^\/([A-Za-z0-9_.-]+)\/([A-Za-z0-9_.-]+)\/(pull|issues)\/([1-9]\d{0,9})$/);
    if (!m) continue;
    const repo = `${m[1]}/${m[2]}`;
    const kind = m[3] === "pull" ? "pr" : "issue";
    // The title is page text: shown in our popovers only, never sent to Home or an agent.
    const title = (a.querySelector(".markdown-title") ?? a).textContent?.replace(/\s+/g, " ").trim().slice(0, 300) || null;
    return { repo, number: Number(m[4]), kind, url: `https://github.com/${repo}/${m[3]}/${m[4]}`, title };
  }
  return null;
}

/** GitHub's own checkbox of a list row is checked (read only: L4 never changes it). */
export function rowChecked(row: Element): boolean {
  return all('input[type="checkbox"]', row).some((c) => (c as HTMLInputElement).checked) || all('[role="checkbox"][aria-checked="true"]', row).length > 0;
}

/** Where a row's chip goes: the end of its metadata line (number, author, time), else the end of the row. */
export function rowSlot(row: Element): { el: Element; place: InsertPosition } {
  const meta = row.querySelector('[data-testid="timestamp-container"], [data-testid="issue-activity-timestamp"]')?.parentElement;
  if (meta && row.contains(meta)) return { el: meta, place: "beforeend" };
  const classic = row.querySelector(".opened-by")?.parentElement;
  if (classic) return { el: classic, place: "beforeend" };
  const actions = row.querySelector(".notification-list-item-actions");
  if (actions) return { el: actions, place: "beforebegin" };
  return { el: row, place: "beforeend" };
}

/** Keep notification controls beside the repository/number, never in a new row below the title.
 * Skip unknown layouts instead of making the notification taller. */
export function notificationSlot(row: Element): { el: Element; place: InsertPosition } | null {
  const meta = row.querySelector(".notification-list-item-link .f6, .notification-list-item-link .text-small");
  return meta ? { el: meta, place: "beforeend" } : null;
}

/** The comment element of a comment's header actions (`comments` anchor): what carries its anchor ID. */
export function commentOf(actions: Element): { id: string; root: Element } | null {
  const classic = actions.closest('.timeline-comment-group[id^="issuecomment-"], .review-comment[id^="discussion_r"]');
  if (classic) return { id: classic.id, root: classic };
  const header = actions.closest('[data-testid="comment-header"][id^="issuecomment-"]');
  if (header) return { id: header.id, root: header.closest('[data-testid^="comment-viewer-outer-box"]') ?? header.parentElement ?? header };
  return null;
}

/** The element a comment's live card goes after, from its anchor ID. */
export function commentRoot(id: string): Element | null {
  const el = document.getElementById(id);
  if (!el) return null;
  return el.closest('.timeline-comment-group, .review-comment, [data-testid^="comment-viewer-outer-box"]') ?? el;
}

/** The comment's text container (to see whether it mentions the owner; never read as context). */
export function commentBody(root: Element): Element | null {
  return root.querySelector('.comment-body, [data-testid="markdown-body"], .markdown-body');
}

/** Every match of a list anchor, recorded like `findAnchor`. */
export function findAnchors(name: AnchorName, p: Params): { els: Element[]; place: InsertPosition; via: Via } | null {
  const anchor: Anchor = ANCHORS[name];
  for (const s of anchor.strategies) {
    let els: Element[] = [];
    try { els = s.all ? s.all(p) : [s.find(p)].filter((x): x is Element => !!x); } catch { els = []; }
    els = els.filter((e) => e.isConnected);
    if (els.length) {
      page.results.set(name, s.via);
      return { els, place: s.place ?? anchor.place, via: s.via };
    }
  }
  if (!page.results.get(name)) page.results.set(name, null);
  return null;
}

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
