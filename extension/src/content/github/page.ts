// The page kind of a github.com URL, from the URL alone (the same rules as Home's `launchpad_github::ext::parse_page`,
// so the layer knows what to look for before Home answers). IDs and numbers come from the URL, never from page text.

import type { ExtPageKind } from "@/gen/ExtPageKind";

export type Page = { kind: ExtPageKind; repo: string | null; number: number | null; url: string };

/** GitHub's top-level paths that are not owners. */
const RESERVED = new Set(["about", "account", "apps", "codespaces", "collections", "contact", "customer-stories", "dashboard", "enterprise", "enterprises", "explore", "features", "issues", "login", "logout", "marketplace", "new", "notifications", "organizations", "orgs", "pricing", "pulls", "readme", "search", "security", "sessions", "settings", "signup", "site", "sponsors", "stars", "team", "topics", "trending", "users", "watching"]);
const NAME = /^[A-Za-z0-9_.-]{1,100}$/;

export function pageOf(href: string): Page {
  const url = new URL(href);
  const page: Page = { kind: "other", repo: null, number: null, url: href };
  if (url.origin !== "https://github.com") return page;
  const [owner, name, ...rest] = url.pathname.split("/").filter(Boolean);
  if (owner === "notifications") return { ...page, kind: "notifications" };
  if (!owner || !name || RESERVED.has(owner.toLowerCase()) || !NAME.test(owner) || !NAME.test(name) || name === "." || name === "..") return page;
  page.repo = `${owner}/${name.replace(/\.git$/, "")}`;
  const num = (s: string | undefined) => (s && /^[1-9]\d{0,9}$/.test(s) ? Number(s) : null);
  const [a, b, c] = rest;
  if (!a || a === "tree" || a === "blob") page.kind = "repo";
  else if (a === "pulls") page.kind = "pulls";
  else if (a === "issues" && num(b)) { page.kind = "issue"; page.number = num(b); }
  else if (a === "issues") page.kind = "issues";
  else if (a === "pull" && num(b)) {
    page.number = num(b);
    page.kind = c === "files" || c === "changes" ? "pr_files" : c === "commits" ? "pr_commits" : "pr_conversation";
  } else if (a === "actions" && b === "runs" && num(c)) page.kind = "actions_run";
  else if (a === "compare") page.kind = "compare";
  else if (a === "releases" && b === "new") page.kind = "new_release";
  return page;
}
