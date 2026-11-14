// Changes of a Local PR, through the shared DiffReview. Viewed markers live in review_state (`viewed:`); line notes
// go to the Local PR's notes for the agent (`onAddNote`).

import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useEffect, useState, type ReactNode } from "react";
import { Icon } from "@/components/Icon";
import { api } from "@/lib/api";
import { useReviewState, viewedKey } from "@/lib/reviewState";
import type { Diff } from "@/components/diff/types";
import { DiffReview, type LineNote, type PhoneFileView } from "@/components/diff/DiffReview";

export function FilesChanged({ id, head, notes, onAddNote, file: addressed, phoneFile, phoneBar, toolbar, focusLine }: {
  /** The Local PR. */
  id: string;
  head: string;
  /** Notes for the agent, shown as line markers. */
  notes?: LineNote[];
  onAddNote?: (note: LineNote) => void;
  /** The file from the address (`?file=`): the selection when `phoneFile` keeps it there, else the first one shown. */
  file?: string;
  /** Phone: the file view kept in the address by the host. */
  phoneFile?: Omit<PhoneFileView, "file">;
  phoneBar?: ReactNode;
  toolbar?: ReactNode;
  focusLine?: number | null;
}) {
  const q = useQuery({
    queryKey: ["predrafts", id, "diff", head],
    queryFn: () => api.get<Diff>(`/api/predrafts/${id}`),
    staleTime: Infinity,
  });
  const [own, setPath] = useState(addressed);
  useEffect(() => { if (addressed) setPath(addressed); }, [addressed]);
  const path = phoneFile ? addressed : own;
  // Viewed markers live in the daemon so every device shares review progress.
  const viewed = useReviewState<string[]>(viewedKey(id, 0, head), []);
  const files = q.data?.files ?? [];
  const file = files.find((f) => f.filename === path) ?? files[0];
  const fileActions = (p: string) => {
    const f = files.find((x) => x.filename === p);
    if (!f || !q.data?.project_id) return null;
    const rev = f.status === "removed" ? q.data.base : q.data.head;
    return <Link className="btn btn-sm no-underline" to="/projects/$id/code" params={{ id: q.data.project_id }} search={{ rev, path: f.filename }}><Icon name="folder" size={12} />View file</Link>;
  };
  return <DiffReview
    files={files.map((f) => ({ path: f.filename, previous: f.previous_filename, status: f.status, additions: f.additions, deletions: f.deletions }))}
    selected={file?.filename}
    onSelect={setPath}
    patch={file ? file.patch ?? "" : undefined}
    loading={q.isPending}
    error={q.error}
    emptyPatch="Binary file or patch unavailable. Open the full file to inspect it."
    language={file?.language}
    status={q.data ? "Local git diff" : undefined}
    viewed={(p) => viewed.value.includes(p)}
    onToggleViewed={(p) => viewed.set((s) => s.includes(p) ? s.filter((n) => n !== p) : [...s, p])}
    focusLine={focusLine}
    toolbar={toolbar}
    draftKey={`line-draft:${id}#0@${head}`}
    notes={notes ?? []}
    onAddNote={onAddNote}
    noteLabel="Note for the agent"
    notePlaceholder="What should the agent change here?"
    fileActions={fileActions}
    phoneView={phoneFile && { ...phoneFile, file: !!addressed }}
    phoneBar={phoneBar}
  />;
}
