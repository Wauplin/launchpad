import type { GitHubRef } from "@/gen/GitHubRef";
import type { PullRequestRef } from "@/gen/PullRequestRef";

const PR_LINK = /https?:\/\/github\.com\/([\w.-]+\/[\w.-]+)\/pull\/(\d+)/gi;

/** The single pull request of `repos` that a draft refers to, from its context or links pasted in the prompt. */
export function pullRequestOf(text: string, refs: GitHubRef[], repos: (string | null | undefined)[]): PullRequestRef | null {
  const wanted = repos.filter((r): r is string => !!r).map((r) => r.toLowerCase());
  const found = new Map<string, PullRequestRef>();
  const add = (repo: string, number: number) => {
    const canonical = wanted.find((w) => w === repo.toLowerCase());
    if (canonical && Number.isSafeInteger(number) && number > 0) found.set(`${canonical}#${number}`, { repo, number });
  };
  for (const r of refs) {
    const link = [...r.url.matchAll(PR_LINK)][0];
    if (link) add(link[1], Number(link[2]));
    else if (r.kind === "pr" && r.number != null) add(r.repo, r.number);
  }
  for (const m of text.matchAll(PR_LINK)) add(m[1], Number(m[2]));
  return found.size === 1 ? [...found.values()][0] : null;
}
