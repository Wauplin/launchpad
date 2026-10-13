import { openExternal } from "@/lib/platform";
import DOMPurify from "dompurify";
import { memo, useMemo } from "react";
import { useComposer } from "@/stores/composer";
import type { GitHubRef } from "@/gen/GitHubRef";
import type { Item, ListItem } from "./types";
export const itemPath = (repo: string, n: number) => `/api/items/${repo}/${n}`;
export const itemKey = (repo: string, n: number) => ["github-item", repo, n];
export function reference(
  repo: string,
  item: Pick<Item | ListItem, "number" | "title" | "url">,
  kind: string,
): GitHubRef {
  return { repo, number: item.number, title: item.title, url: item.url, kind };
}
export function AddContext({ refItem }: { refItem: GitHubRef }) {
  const open = useComposer((s) => s.open);
  const present = useComposer((s) =>
    s.spec.github_refs.some((g) => g.url === refItem.url),
  );
  if (!open) return null;
  return (
    <button
      className="btn btn-sm"
      disabled={present}
      onClick={() =>
        useComposer
          .getState()
          .update((s) => ({ ...s, github_refs: [...s.github_refs, refItem] }))
      }
    >
      {present ? "Added" : "+ Add"}
    </button>
  );
}
export function openGitHub(url: string) {
  openExternal(url);
}
export const GitHubHTML = memo(function GitHubHTML({ html }: { html: string }) {
  const safe = useMemo(
    () =>
      DOMPurify.sanitize(html, {
        FORBID_TAGS: ["style", "form", "iframe"],
        FORBID_ATTR: ["style"],
      }),
    [html],
  );
  return (
    <div
      className="markdown-body lp-gh-html"
      onClick={(e) => {
        const a = (e.target as HTMLElement).closest("a");
        if (a) {
          e.preventDefault();
          openGitHub(a.href);
        }
      }}
      dangerouslySetInnerHTML={{ __html: safe }}
    />
  );
});
