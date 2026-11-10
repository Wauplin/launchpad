import { Link } from "@tanstack/react-router";
import { usePendingReview } from "./pendingReview";
export function ReviewReady({ repo, number }: { repo: string; number: number }) {
  const draft = usePendingReview(repo, number);
  const [owner, name] = repo.split("/");
  return <Link className="btn btn-sm no-underline" to="/github/$owner/$repo/$number" params={{ owner, repo: name, number: String(number) }} search={{ tab: "files", draft: "1" }} onClick={() => window.dispatchEvent(new CustomEvent("open-review", { detail: { repo, number } }))}>Review ready · {draft.value.comments.length} comments</Link>;
}
