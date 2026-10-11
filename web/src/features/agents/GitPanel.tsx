import { useQuery } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useRef, useState } from "react";
import { api } from "@/lib/api";
import type { SessionGitStatus as Status } from "@/gen/SessionGitStatus";
import type { SessionGitDiff as Diff } from "@/gen/SessionGitDiff";
const PatchRow = memo(function PatchRow({ text }: { text: string }) {
  return <div className={`h-5 whitespace-pre px-2 font-mono text-xs ${text.startsWith("+") ? "bg-[#e6ffed]" : text.startsWith("-") ? "bg-[#ffeef0]" : text.startsWith("@@") ? "bg-select" : ""}`}>{text || " "}</div>;
});
export function GitPanel({ sessionId, initialMain = false }: { sessionId: string; initialMain?: boolean }) {
  const [path, setPath] = useState<string | null>(null);
  const [main, setMain] = useState(initialMain);
  const [staged, setStaged] = useState(false);
  const status = useQuery({ queryKey:["session-git",sessionId],queryFn:() => api.get<Status>(`/api/sessions/${sessionId}/git`),refetchInterval:5000 });
  const files = (main ? status.data?.main_files : status.data?.files) ?? [];
  const selected = files.find((f) => f.path === path)?.path ?? files[0]?.path;
  const diff = useQuery({ queryKey:["session-git-diff",sessionId,selected,main],queryFn:() => api.get<Diff>(`/api/sessions/${sessionId}/git-diff?path=${encodeURIComponent(selected!)}&main=${main}`),enabled:!!selected,refetchInterval:5000 });
  const listRef = useRef<HTMLDivElement>(null);
  const codeRef = useRef<HTMLDivElement>(null);
  const text = (staged && !main ? diff.data?.staged : diff.data?.unstaged) ?? "";
  const rows = text.split("\n");
  const list = useVirtualizer({ count: files.length, getScrollElement:() => listRef.current,estimateSize:() => 28,overscan:4,useFlushSync:false });
  const lines = useVirtualizer({ count: rows.length,getScrollElement:() => codeRef.current,estimateSize:() => 20,overscan:6,useFlushSync:false });
  return <div className="flex min-h-0 flex-1 flex-col">
    <div className="flex flex-wrap items-center gap-2 border-b border-line p-2 text-xs"><b>{status.data?.branch ?? "Git status"}</b><span>{files.length} changed files</span><button className="btn btn-sm" onClick={() => { void status.refetch(); void diff.refetch(); }}>Refresh</button><button className={`btn btn-sm ${!staged && !main ? "bg-select!" : ""}`} onClick={() => { setStaged(false); setMain(false); }}>Working tree</button><button className={`btn btn-sm ${staged && !main ? "bg-select!" : ""}`} onClick={() => { setStaged(true); setMain(false); }}>Staged</button><button className={`btn btn-sm ${main ? "bg-select!" : ""}`} disabled={!status.data?.main_changes} onClick={() => setMain(true)}>Compared with main</button></div>
    {status.error && (/not a git repository/i.test(status.error.message) ? <p className="p-3 text-xs text-ink-3">This folder is not a Git repository.</p> : <p className="p-3 text-xs text-gh-closed">{status.error.message}</p>)}
    {status.isPending && <p className="p-3 text-xs">Reading Git status…</p>}
    {status.data && !files.length && <p className="p-3 text-xs text-ink-3">{main ? "No tracked changes compared with main." : "Working tree clean."}</p>}
    {!!files.length && <div className="flex min-h-0 flex-1">
      <div ref={listRef} className="w-[160px] shrink-0 overflow-auto border-r border-line"><div style={{ height:list.getTotalSize(),position:"relative" }}>{list.getVirtualItems().map((r) => { const f = files[r.index]; return <button key={f.path} className={`flex h-7 w-full gap-2 px-2 text-left text-xs ${selected === f.path ? "bg-select" : "hover:bg-sunk"}`} title={`${f.previous ? `${f.previous} → ` : ""}${f.path}`} style={{position:"absolute",left:0,top:0,transform:`translateY(${r.start}px)`}} onClick={() => { setPath(f.path); lines.scrollToOffset(0); }}><span className="font-mono whitespace-pre text-teal-ink">{f.status}</span><span className="truncate">{f.path}</span></button>; })}</div></div>
      <div className="flex min-w-0 flex-1 flex-col"><div className="truncate border-b border-line p-2 font-mono text-xs" title={selected}>{selected}</div>
        {diff.error && <p className="p-2 text-xs text-gh-closed">{diff.error.message}</p>}
        {diff.isPending && <p className="p-2 text-xs">Loading diff…</p>}
        {diff.data?.truncated && <p className="p-2 text-xs text-ink-3">Large diff: showing its beginning and end.</p>}
        {diff.data && !text && <p className="p-2 text-xs text-ink-3">No {main ? "changes compared with main" : staged ? "staged" : "working tree"} changes for this file.</p>}
        <div ref={codeRef} className="min-h-0 flex-1 overflow-auto"><div style={{height:lines.getTotalSize(),position:"relative",minWidth:"100%",width:"max-content"}}>{lines.getVirtualItems().map((r) => <div key={r.index} style={{position:"absolute",top:0,left:0,minWidth:"100%",transform:`translateY(${r.start}px)`}}><PatchRow text={rows[r.index]} /></div>)}</div></div>
      </div>
    </div>}
  </div>;
}
