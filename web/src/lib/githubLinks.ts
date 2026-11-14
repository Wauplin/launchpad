// Canonical links to GitHub issues and pull requests. Launchpad does not show remote items: every link to one
// opens github.com in a new tab.

/** GitHub's characters for an owner or repository name, never `.` or `..`. */
export function validName(name: string): boolean {
  return /^[A-Za-z0-9._-]+$/.test(name) && name !== "." && name !== "..";
}

/** `owner/repo` split and checked, else null. */
export function splitRepo(repo: string): [string, string] | null {
  const parts = repo.split("/");
  if (parts.length !== 2 || !validName(parts[0]) || !validName(parts[1])) return null;
  return [parts[0], parts[1]];
}

export type GitHubLinkOptions = {
  /** `pr` gives `/pull/N`; anything else `/issues/N` (GitHub redirects a PR's number there). */
  kind?: string | null;
  /** An anchor such as `issuecomment-123` or `discussion_r456` (with or without `#`). */
  anchor?: string | null;
  /** Open the pull request's Files changed tab. */
  files?: boolean;
};

const isPr = (kind?: string | null) => !!kind && /^(pr|pull|pullrequest|pull_request)$/i.test(kind);

/** The github.com URL of an issue or pull request, or null when the repository or number is invalid. */
export function githubItemUrl(repo: string, number: number, opts: GitHubLinkOptions = {}): string | null {
  const parts = splitRepo(repo);
  if (!parts || !Number.isSafeInteger(number) || number <= 0) return null;
  const pr = isPr(opts.kind) || !!opts.files;
  let url = `https://github.com/${parts[0]}/${parts[1]}/${pr ? "pull" : "issues"}/${number}`;
  if (opts.files) url += "/files";
  const anchor = opts.anchor?.replace(/^#/, "");
  if (anchor && /^[A-Za-z0-9_-]+$/.test(anchor)) url += `#${anchor}`;
  return url;
}

/** The github.com URL of a repository page (`pulls`, `issues`, or the repository itself). */
export function githubRepoUrl(repo: string, page?: "pulls" | "issues"): string | null {
  const parts = splitRepo(repo);
  if (!parts) return null;
  return `https://github.com/${parts[0]}/${parts[1]}${page ? `/${page}` : ""}`;
}

/** Opens an issue or pull request on github.com in a new tab. */
export function openGitHubItem(repo: string, number: number, opts: GitHubLinkOptions = {}) {
  const url = githubItemUrl(repo, number, opts);
  if (url) window.open(url, "_blank", "noopener,noreferrer");
}

/** Opens a github.com URL in a new tab (only `https://github.com/` addresses). */
export function openGitHub(url: string) {
  if (/^https:\/\/github\.com\//.test(url)) window.open(url, "_blank", "noopener,noreferrer");
}

/** The github.com page of a GitHub suggestion's item: the pull request for a review, the answered comment for a
 * reply, else the issue or pull request. */
export function suggestionItemUrl(s: { kind: string; target: { kind: string; repo?: string; number?: number; reply_to?: { kind: string; id: number } } }): string | null {
  const t = s.target;
  if (t.kind !== "git_hub" || !t.repo || !t.number) return null;
  const reply = t.reply_to;
  const anchor = reply ? (reply.kind === "review_comment" ? `discussion_r${reply.id}` : `issuecomment-${reply.id}`) : null;
  return githubItemUrl(t.repo, t.number, { kind: s.kind === "review" || reply?.kind === "review_comment" ? "pr" : null, anchor });
}
