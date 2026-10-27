// "Review with agent": opens a launch draft that asks an agent for a local suggested review. Used by the PR page,
// the Inbox and the Review page. Nothing is posted: the suggestion waits for the owner's click on the PR page.

import type { Project } from "@/gen/Project";
import { useComposer } from "@/stores/composer";

export function projectForRepo(projects: Project[], repo: string) {
  const r = repo.toLowerCase();
  return projects.find((p) => p.gh_repo?.toLowerCase() === r) ?? projects.find((p) => p.gh_upstream?.toLowerCase() === r);
}

export function reviewPrompt(repo: string, number: number, head?: string | null) {
  const at = head ? ` at commit ${head}` : "";
  const flag = head ? `--head ${head}` : "--head <the commit you inspected>";
  return `Review ${repo}#${number}${at}. Inspect the changes and relevant context, summarize concrete findings, and save a local suggested review using launchpad suggest review --repo ${repo} --number ${number} ${flag} --body-file <file> with your session id. Use --event request-changes for blocking findings, otherwise a comment review. Do not post to GitHub or change the code.`;
}

/** Opens the composer with the review prompt and the PR as context. The owner still clicks Launch. */
export function reviewWithAgent({ projects, repo, number, title, url, head }: { projects: Project[]; repo: string; number: number; title: string; url: string; head?: string | null }) {
  const c = useComposer.getState();
  c.start({ projectId: projectForRepo(projects, repo)?.id ?? null, githubRefs: [{ repo, number, title, url, kind: "pr" }] });
  c.update((s) => ({ ...s, text: reviewPrompt(repo, number, head) }));
}
