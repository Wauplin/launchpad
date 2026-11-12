// The line comments of an agent's suggested review: shown on its card and in the review panel, posted with the review.

import type { LineComment } from "@/gen/LineComment";
import type { LineNote } from "@/features/github/DiffReview";
import { Markdown } from "@/components/Markdown";

/** `src/a.rs:10–12`, with "(removed line)" for the old side. */
export const commentPlace = (c: LineComment) => `${c.path}:${c.start_line ? `${c.start_line}–` : ""}${c.line}${c.side === "LEFT" ? " (removed line)" : ""}`;

/** A suggested line comment as a line comment of the pending review, at the commit the agent reviewed. */
export function asLineNote(c: LineComment, head: string | null): LineNote {
  return { path: c.path, line: c.line, side: c.side === "LEFT" ? "old" : "new", code: "", text: c.body, ...(head ? { head } : {}), ...(c.start_line ? { startLine: c.start_line } : {}) };
}

export function LineComments({ comments, onRemove, disabled }: { comments: LineComment[]; onRemove?: (index: number) => void; disabled?: boolean }) {
  if (!comments.length) return null;
  return <div className="grid gap-2" aria-label="Line comments">
    <span className="text-[12px] font-semibold">{comments.length === 1 ? "1 line comment" : `${comments.length} line comments`}</span>
    {comments.map((c, i) => <div key={i} className="grid min-w-0 gap-1 border-l border-line pl-2 text-[12px]">
      <span className="truncate font-mono text-teal-ink" title={commentPlace(c)}>{commentPlace(c)}</span>
      <Markdown text={c.body} />
      {onRemove && <button className="btn btn-sm w-fit" disabled={disabled} onClick={() => onRemove(i)}>Remove</button>}
    </div>)}
  </div>;
}
