import type { IconName } from "@/components/Icon";
export const itemIcon = (kind: string, state: string): { name: IconName; color: string } => {
  if (kind === "pr") {
    if (state === "merged") return { name: "pr-merged", color: "text-gh-merged" };
    if (state === "draft") return { name: "pr-draft", color: "text-gh-draft" };
    if (state === "closed") return { name: "pr", color: "text-gh-closed" };
    return { name: "pr", color: "text-gh-open" };
  }
  if (state === "closed") return { name: "issue-done", color: "text-gh-merged" };
  return { name: "issue", color: "text-gh-open" };
};

