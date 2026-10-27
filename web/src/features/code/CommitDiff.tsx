import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { Icon } from "@/components/Icon";
import { DiffReview } from "@/features/github/DiffReview";
import { Empty } from "@/components/ui";
import type { DiffFile } from "@/features/github/types";
import { sourceParams } from "./lib";

export type CommitChanges = { sha: string; subject: string; parent: string | null; files: DiffFile[] };

export function CommitDiff({ root, rev, path, select, openFile }: { root: string; rev: string; path?: string; select: (path: string) => void; openFile: (rev: string, path: string) => void }) {
  const q = useQuery({
    queryKey: ["code-commit-diff", root, rev],
    queryFn: () => api.get<CommitChanges>(`/api/code/commit-diff?${sourceParams(root, rev)}`),
    staleTime: /^(?:[0-9a-f]{40}|[0-9a-f]{64})$/i.test(rev) ? Infinity : 0,
    retry: false,
  });
  const files = q.data?.files ?? [];
  const file = files.find((f) => f.filename === path) ?? files[0];
  const patch = useQuery({
    queryKey: ["code-commit-diff-file", root, q.data?.sha, file?.filename],
    queryFn: () => api.get<DiffFile>(`/api/code/commit-diff/file?${sourceParams(root, q.data!.sha)}&path=${encodeURIComponent(file!.filename)}`),
    enabled: !!q.data && !!file,
    staleTime: Infinity,
    retry: false,
  });
  const noViews = () => false;
  return <section className="flex min-h-0 flex-1 flex-col">
    <div className="flex flex-wrap items-center gap-2 border-b border-line p-3">
      <b className="min-w-0 break-words">{q.data?.subject || "Commit changes"}</b>
      {q.data && <span className="text-xs text-ink-3">{files.length} files changed · {q.data.parent ? `Compared with first parent ${q.data.parent.slice(0, 7)}` : "Root commit · all additions"}</span>}
    </div>
    {q.data && !files.length ? <Empty icon="commit">This commit has no file changes.</Empty> : <DiffReview
      files={files.map((f) => ({ path: f.filename, previous: f.previous_filename, status: f.status, additions: f.additions, deletions: f.deletions }))}
      selected={file?.filename}
      onSelect={select}
      patch={patch.data ? patch.data.patch ?? "" : undefined}
      loading={q.isPending || patch.isPending}
      error={q.error ?? patch.error}
      emptyPatch="No text patch: this is a binary file, a rename without content changes, or a file mode change."
      language={patch.data?.language}
      viewed={noViews}
      fileActions={(path) => { const f = files.find((x) => x.filename === path); return f ? <button className="btn btn-sm" onClick={() => openFile(f.status === "removed" ? q.data!.parent! : q.data!.sha, f.filename)}><Icon name="folder" size={12} />Open in Files</button> : null; }}
    />}
  </section>;
}
