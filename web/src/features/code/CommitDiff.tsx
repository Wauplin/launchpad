import { useQuery } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useRef, useState } from "react";
import { api } from "@/lib/api";
import { FilePatch } from "@/features/github/FilesChanged";
import type { DiffFile } from "@/features/github/types";
import { sourceParams } from "./lib";

export type CommitChanges = { sha: string; subject: string; parent: string | null; files: DiffFile[] };

const CommitFileRow = memo(function CommitFileRow({ file, active, select }: { file: DiffFile; active: boolean; select: (path: string) => void }) {
  return <button className={`flex h-10 w-full items-center gap-2 px-3 text-left text-xs ${active ? "bg-select" : "hover:bg-sunk"}`} onClick={() => select(file.filename)} title={file.previous_filename ? `${file.previous_filename} → ${file.filename}` : file.filename}>
    <span className="min-w-0 flex-1 truncate font-mono">{file.filename}</span>
    {file.additions != null && <span className="shrink-0 tabular-nums"><span className="text-gh-open">+{file.additions}</span> <span className="text-gh-closed">−{file.deletions}</span></span>}
    <span className="text-ink-3">{file.status.slice(0, 1).toUpperCase()}</span>
  </button>;
});

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
  const [split, setSplit] = useState(() => localStorage.getItem("diff-view") === "split");
  const ref = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({ count: files.length, getScrollElement: () => ref.current, estimateSize: () => 40, overscan: 3, useFlushSync: false });
  return <section className="flex min-h-0 flex-1 flex-col">
    <div className="flex flex-wrap items-center gap-2 border-b border-line p-3">
      <b className="min-w-0 break-words">{q.data?.subject || "Commit changes"}</b>
      {q.data && <span className="text-xs text-ink-3">{files.length} files changed · {q.data.parent ? `Compared with first parent ${q.data.parent.slice(0, 7)}` : "Root commit · all additions"}</span>}
      <button className="btn btn-sm ml-auto" onClick={() => { localStorage.setItem("diff-view", split ? "unified" : "split"); setSplit(!split); }}>{split ? "Unified view" : "Split view"}</button>
    </div>
    {q.isPending && <p className="p-4 text-ink-3">Loading commit changes…</p>}
    {q.error && <p className="p-4 text-gh-closed">{String(q.error)}</p>}
    {q.data && !files.length && <p className="p-4 text-ink-3">This commit has no file changes.</p>}
    {!!files.length && <div className="flex min-h-0 flex-1 flex-col sm:flex-row">
      <div ref={ref} className="h-36 shrink-0 overflow-auto border-b border-line sm:h-auto sm:w-[240px] sm:border-r sm:border-b-0">
        <div className="relative" style={{ height: v.getTotalSize() }}>
          {v.getVirtualItems().map((row) => <div key={files[row.index].filename} className="absolute w-full" style={{ top: row.start }}><CommitFileRow file={files[row.index]} active={file === files[row.index]} select={select} /></div>)}
        </div>
      </div>
      <div className="min-h-0 min-w-0 flex-1 overflow-auto">
        {file && <div className="flex flex-wrap items-center gap-2 p-3">
          <b className="min-w-0 break-all font-mono text-xs">{file.previous_filename && `${file.previous_filename} → `}{file.filename}</b>
          <span className="text-xs text-ink-3">{file.status}</span>
          <button className="btn btn-sm ml-auto" onClick={() => openFile(file.status === "removed" ? q.data!.parent! : q.data!.sha, file.filename)}>View file</button>
        </div>}
        {patch.isPending && <p className="p-4 text-ink-3">Loading patch…</p>}
        {patch.error && <p className="p-4 text-gh-closed">{String(patch.error)}</p>}
        {patch.data && <FilePatch file={patch.data} split={split} emptyMessage="No text patch: this is a binary file, a rename without content changes, or a file mode change." />}
      </div>
    </div>}
  </section>;
}
