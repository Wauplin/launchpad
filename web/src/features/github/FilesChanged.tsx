// Changes of a GitHub PR or a Local PR, through the shared DiffReview. Viewed markers live in review_state
// (`viewed:`). PR line notes become line comments of your next review (`pr-notes:`, local until Submit review);
// Local PR line notes go to the Local PR's notes for the agent (`onAddNote`).

import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useState } from "react";
import { Icon } from "@/components/Icon";
import { api } from "@/lib/api";
import { useReviewState, viewedKey } from "@/lib/reviewState";
import type { Diff, DiffFile } from "./types";
import { itemPath, openGitHub } from "./lib";
import { DiffReview, type LineNote } from "./DiffReview";
export { FilePatch } from "./DiffReview";

export const prNotesKey = (repo: string, number: number) => `pr-notes:${repo}#${number}`;
/** Line comments of the next review on a PR, kept locally until Submit review. */
export const usePrNotes = (repo: string, number: number) => useReviewState<LineNote[]>(prNotesKey(repo, number), []);

export function FilesChanged({ repo, number, head, base, local, notes, onAddNote }: {
  repo: string;
  number: number;
  head: string;
  base?: string;
  local?: { id: string; root: string };
  /** Local PRs: notes for the agent, shown as line markers. PRs use `pr-notes:` review state. */
  notes?: LineNote[];
  onAddNote?: (note: LineNote) => void;
}) {
  const q = useQuery({
    queryKey: local ? ["predrafts", local.id, "diff", head] : ["pr-diff", repo, number, base, head],
    queryFn: () => local ? api.get<Diff>(`/api/predrafts/${local.id}`) : api.post<Diff>(`${itemPath(repo, number)}/diff`),
    staleTime: Infinity,
  });
  const [path, setPath] = useState<string>();
  // Viewed markers live in the daemon so every device shares review progress.
  const viewed = useReviewState<string[]>(viewedKey(local?.id ?? repo, number, head), []);
  const prNotes = useReviewState<LineNote[]>(local ? "pr-notes:unused" : prNotesKey(repo, number), []);
  const files = q.data?.files ?? [];
  const listed = files.find((f) => f.filename === path) ?? files[0];
  // PR file lists come without patches: load the selected file's patch on demand.
  const lazy = !!listed && listed.patch === undefined && !!listed.has_patch && !local;
  const patch = useQuery({
    queryKey: ["pr-diff-file", repo, number, q.data?.base, q.data?.head, q.data?.project_id, listed?.filename],
    queryFn: () => api.get<DiffFile>(`${itemPath(repo, number)}/diff/file?${new URLSearchParams({ base: q.data!.base, head: q.data!.head, project: q.data!.project_id ?? "api", path: listed!.filename })}`),
    enabled: lazy,
    staleTime: Infinity,
  });
  const file = lazy ? patch.data : listed;
  const noteList = local ? notes ?? [] : prNotes.value.filter((n) => !n.head || n.head === head);
  const fileActions = (p: string) => {
    const f = files.find((x) => x.filename === p);
    if (!f || !q.data) return null;
    const rev = f.status === "removed" ? q.data.base : q.data.head;
    return q.data.project_id
      ? <Link className="btn btn-sm no-underline" to="/projects/$id/code" params={{ id: q.data.project_id }} search={{ root: local?.root, rev, path: f.filename }}><Icon name="folder" size={12} />View file</Link>
      : <button className="btn btn-sm" onClick={() => openGitHub(`https://github.com/${repo}/blob/${rev}/${f.filename.split("/").map(encodeURIComponent).join("/")}`)}><Icon name="ext" size={12} />View on GitHub</button>;
  };
  return <DiffReview
    files={files.map((f) => ({ path: f.filename, previous: f.previous_filename, status: f.status, additions: f.additions, deletions: f.deletions }))}
    selected={listed?.filename}
    onSelect={setPath}
    patch={file ? file.patch ?? "" : undefined}
    loading={q.isPending || (lazy && patch.isPending)}
    error={q.error ?? (lazy ? patch.error : null)}
    emptyPatch="Binary file or patch unavailable. Open the full file or GitHub to inspect it."
    language={file?.language}
    status={q.data ? q.data.fallback ? "GitHub patches (large patches may be truncated)" : "Local git diff" : undefined}
    viewed={(p) => viewed.value.includes(p)}
    onToggleViewed={(p) => viewed.set((s) => s.includes(p) ? s.filter((n) => n !== p) : [...s, p])}
    notes={noteList}
    onAddNote={local ? onAddNote : (n) => prNotes.set((all) => [...all, { ...n, head }])}
    noteLabel={local ? "Note for the agent" : "Line comment"}
    notePlaceholder={local ? "What should the agent change here?" : "Comment for this line, posted with your next review"}
    fileActions={fileActions}
  />;
}
