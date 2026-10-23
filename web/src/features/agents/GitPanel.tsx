// Changes view of a session (read-only Git): file list, one file's diff, viewed markers and line notes for the agent.
// Viewed markers and unsent notes are review state on the daemon (`session-viewed:`, `session-notes:`), so a review
// started on the phone continues on the laptop. Notes go to the agent as one message the owner reads before sending.
// The phone shows the file list or one file at a time.

import { useQuery } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useMemo, useRef, useState } from "react";
import { api } from "@/lib/api";
import { useReviewState } from "@/lib/reviewState";
import { usePhone } from "@/lib/phone";
import { Icon } from "@/components/Icon";
import { toast, toastError } from "@/components/Toast";
import type { SessionGitStatus as Status } from "@/gen/SessionGitStatus";
import type { SessionGitHistory as History } from "@/gen/SessionGitHistory";
import type { SessionGitDiff as Diff } from "@/gen/SessionGitDiff";

type Note = { path: string; line: number | null; code: string; text: string };
type Line = { text: string; number: number | null };

/** Diff lines with the new-file line number (old-file number for deleted lines). */
function numbered(text: string): Line[] {
  let oldLine = 0;
  let newLine = 0;
  return text.split("\n").map((t) => {
    const hunk = /^@@ -(\d+)(?:,\d+)? \+(\d+)/.exec(t);
    if (hunk) { oldLine = Number(hunk[1]); newLine = Number(hunk[2]); return { text: t, number: null }; }
    if (t.startsWith("+++") || t.startsWith("---") || t.startsWith("diff ") || t.startsWith("index ")) return { text: t, number: null };
    if (t.startsWith("+")) return { text: t, number: newLine++ };
    if (t.startsWith("-")) return { text: t, number: oldLine++ };
    if (t.startsWith("\\")) return { text: t, number: null };
    oldLine++;
    return { text: t, number: newLine++ };
  });
}

/** Short content signature: a file stays viewed only while its diff is unchanged. */
function signature(text: string): string {
  let h = 0;
  for (let i = 0; i < text.length; i++) h = (Math.imul(31, h) + text.charCodeAt(i)) | 0;
  return `${text.length}:${h >>> 0}`;
}

/** The exact message sent to the agent for the collected notes. */
export function notesMessage(notes: Note[]): string {
  const lines = ["Review notes on your changes. Please address each one:", ""];
  notes.forEach((n, i) => {
    lines.push(`${i + 1}. \`${n.path}\`${n.line != null ? ` line ${n.line}` : ""}${n.code.trim() ? `: \`${n.code.trim().slice(0, 160)}\`` : ""}`);
    lines.push(`   ${n.text.trim().replace(/\n/g, "\n   ")}`);
  });
  return lines.join("\n");
}

const PatchRow = memo(function PatchRow({ line, wrap, noted, selected, onPick }: { line: Line; wrap: boolean; noted: boolean; selected: boolean; onPick: (() => void) | null }) {
  const t = line.text;
  const tone = t.startsWith("+") ? "bg-diff-add" : t.startsWith("-") ? "bg-diff-del" : t.startsWith("@@") ? "bg-select" : "";
  return <div className={`flex min-h-5 font-mono text-xs ${tone} ${selected ? "outline-2 -outline-offset-2 outline-teal" : ""}`} onClick={onPick ?? undefined}>
    <span className={`w-11 flex-none pr-1.5 text-right text-ink-3 select-none ${noted ? "bg-attn-soft font-bold text-attn" : ""}`}>{noted ? "✎ " : ""}{line.number ?? ""}</span>
    <span className={`px-2 ${wrap ? "min-w-0 flex-1 break-all whitespace-pre-wrap" : "whitespace-pre"}`}>{t || " "}</span>
  </div>;
});

export function GitPanel({ sessionId, initialMain = false, canMessage = true }: { sessionId: string; initialMain?: boolean; canMessage?: boolean }) {
  const phone = usePhone();
  const [path, setPath] = useState<string | null>(null);
  const [showList, setShowList] = useState(true);
  const [main, setMain] = useState(initialMain);
  const [mode, setMode] = useState<"auto" | "live" | "committed">("auto");
  const [commit, setCommit] = useState("");
  const history = useQuery({ queryKey: ["session-git-commits", sessionId, ""], queryFn: () => api.get<History | null>(`/api/sessions/${sessionId}/git-commits`), refetchInterval: 5000 });
  const commitHistory = useQuery({ queryKey: ["session-git-commits", sessionId, commit], queryFn: () => api.get<History | null>(`/api/sessions/${sessionId}/git-commits?commit=${commit}`), enabled: !!commit, refetchInterval: commit ? 5000 : false });
  const selectedHistory = commit ? commitHistory.data : history.data;
  const [staged, setStaged] = useState(false);
  const [wrap, setWrap] = useState(phone);
  const status = useQuery({ queryKey: ["session-git", sessionId], queryFn: () => api.get<Status>(`/api/sessions/${sessionId}/git`), refetchInterval: 5000 });
  const committed = mode === "committed" || (mode === "auto" && !status.isPending && !status.data?.files.length && !!history.data?.commits.length);
  const files = (committed ? selectedHistory?.files : main ? status.data?.main_files : status.data?.files) ?? [];
  const selected = files.find((f) => f.path === path)?.path ?? files[0]?.path;
  const diff = useQuery({ queryKey: ["session-git-diff", sessionId, selected, main, committed, commit, committed ? selectedHistory?.head : null], queryFn: () => api.get<Diff>(`/api/sessions/${sessionId}/git-diff?path=${encodeURIComponent(selected!)}&main=${main}&committed=${committed}${committed && commit ? `&commit=${commit}` : ""}`), enabled: !!selected, refetchInterval: 5000 });
  const viewed = useReviewState<Record<string, string>>(`session-viewed:${sessionId}`, {});
  const notes = useReviewState<Note[]>(`session-notes:${sessionId}`, []);
  const [picked, setPicked] = useState<number | null>(null);
  const [draft, setDraft] = useState("");
  const [preview, setPreview] = useState(false);
  const [sending, setSending] = useState(false);
  const listRef = useRef<HTMLDivElement>(null);
  const codeRef = useRef<HTMLDivElement>(null);
  const text = (staged && !main && !committed ? diff.data?.staged : diff.data?.unstaged) ?? "";
  const rows = useMemo(() => numbered(text), [text]);
  const scope = committed ? `committed:${selectedHistory?.base}:${selectedHistory?.head}` : main ? "main" : staged ? "staged" : "tree";
  const key = (p: string) => `${scope}:${p}`;
  const viewedCount = files.filter((f) => key(f.path) in viewed.value).length;
  const fileNotes = notes.value.filter((n) => n.path === selected);
  const list = useVirtualizer({ count: files.length, getScrollElement: () => listRef.current, estimateSize: () => (phone ? 44 : 28), overscan: 4, useFlushSync: false });
  const lines = useVirtualizer({ count: rows.length, getScrollElement: () => codeRef.current, estimateSize: () => 20, overscan: 6, useFlushSync: false, measureElement: wrap ? (el) => el.getBoundingClientRect().height : undefined });
  const open = (p: string) => { setPath(p); setPicked(null); setShowList(false); lines.scrollToOffset(0); };
  const toggleViewed = () => {
    if (!selected) return;
    viewed.set((v) => {
      const next = { ...v };
      if (key(selected) in next && next[key(selected)] === signature(text)) delete next[key(selected)];
      else next[key(selected)] = signature(text);
      return next;
    });
  };
  const addNote = () => {
    if (picked === null || !selected || !draft.trim()) return;
    const line = rows[picked];
    notes.set((n) => [...n, { path: selected, line: line.number, code: line.text, text: draft.trim() }]);
    setDraft("");
    setPicked(null);
  };
  const send = async () => {
    setSending(true);
    try {
      await api.post(`/api/sessions/${sessionId}/input`, { text: notesMessage(notes.value) });
      await notes.clear();
      setPreview(false);
      toast("Notes sent to the agent");
    } catch (e) { toastError(e); } finally { setSending(false); }
  };
  const at = files.findIndex((f) => f.path === selected);
  const selectedViewed = !!selected && viewed.value[key(selected)] === signature(text) && !!diff.data;
  const fileList = <div ref={listRef} className={`overflow-auto ${phone ? "min-h-0 flex-1" : "w-[180px] shrink-0 border-r border-line"}`}><div style={{ height: list.getTotalSize(), position: "relative" }}>{list.getVirtualItems().map((r) => {
    const f = files[r.index];
    const done = key(f.path) in viewed.value;
    const count = notes.value.filter((n) => n.path === f.path).length;
    return <button key={f.path} className={`flex w-full items-center gap-2 px-2 text-left text-xs ${phone ? "h-11 border-b border-line-2 text-[13px]" : "h-7"} ${selected === f.path && !phone ? "bg-select" : "hover:bg-sunk"}`} title={`${f.previous ? `${f.previous} → ` : ""}${f.path}`} style={{ position: "absolute", left: 0, top: 0, transform: `translateY(${r.start}px)` }} onClick={() => open(f.path)}>
      <span className="font-mono whitespace-pre text-teal-ink">{f.status}</span><span className={`min-w-0 flex-1 truncate ${done ? "text-ink-3" : ""}`}>{f.path}</span>{count > 0 && <span className="text-attn" title={`${count} note${count === 1 ? "" : "s"}`}>✎{count}</span>}{done && <Icon name="check" size={12} className="text-gh-open" />}
    </button>;
  })}</div></div>;
  return <div className="flex min-h-0 flex-1 flex-col">
    <div className="flex flex-wrap items-center gap-2 border-b border-line p-2 text-xs"><b>{status.data?.branch ?? "Git status"}</b><span>{files.length} changed files</span>
      {files.length > 0 && <span className="flex items-center gap-1.5" title="Files marked viewed (shared with your other devices)"><span className="inline-block h-1.5 w-16 overflow-hidden rounded bg-meter"><i className="block h-full bg-gh-open" style={{ width: `${(viewedCount / files.length) * 100}%` }} /></span>{viewedCount}/{files.length} viewed</span>}
      <button className="btn btn-sm" onClick={() => { void status.refetch(); void history.refetch(); if (commit) void commitHistory.refetch(); if (selected) void diff.refetch(); }}>Refresh</button><button className={`btn btn-sm ${!staged && !main && !committed ? "bg-select!" : ""}`} onClick={() => { setMode("live"); setStaged(false); setMain(false); setPicked(null); }}>Working tree</button><button className={`btn btn-sm ${staged && !main && !committed ? "bg-select!" : ""}`} onClick={() => { setMode("live"); setStaged(true); setMain(false); setPicked(null); }}>Staged</button><button className={`btn btn-sm ${main && !committed ? "bg-select!" : ""}`} disabled={!status.data?.main_changes} onClick={() => { setMode("live"); setMain(true); setPicked(null); }}>Compared with main</button>
      {!!history.data?.commits.length && <button className={`btn btn-sm ${committed ? "bg-select!" : ""}`} onClick={() => { setMode("committed"); setPicked(null); }}>Committed changes</button>}</div>
    {history.data?.merged_into && <p className="border-b border-line bg-chrome px-2 py-2 text-xs">Merged into <b>{history.data.merged_into}</b>. Committed changes remain available for review.</p>}
    {committed && history.data && <label className="flex min-w-0 items-center gap-2 border-b border-line p-2 text-xs">Commits
      <select aria-label="Commit to review" className="field-input min-w-0 flex-1" value={commit} onChange={(e) => { setCommit(e.target.value); setPath(null); setPicked(null); setShowList(true); }}>
        <option value="">All commits ({history.data.commits.length})</option>
        {history.data.commits.map((c) => <option key={c.id} value={c.id}>{c.id.slice(0, 8)} · {c.subject}</option>)}
      </select>
    </label>}
    {commit && commitHistory.error && <p className="p-2 text-xs text-gh-closed">{commitHistory.error.message}. Select All commits to refresh the selection.</p>}
    {history.error && <p className="p-2 text-xs text-gh-closed">Committed changes unavailable: {history.error.message}</p>}
    {notes.value.length > 0 && <div className="flex flex-wrap items-center gap-2 border-b border-attn-line bg-attn-soft px-2 py-1.5 text-xs">
      <b className="text-attn">{notes.value.length} note{notes.value.length === 1 ? "" : "s"} for the agent</b>
      <button className="btn btn-sm btn-primary" disabled={!canMessage} title={canMessage ? "Read the exact message, then send it" : "The agent cannot take a message now"} onClick={() => setPreview(true)}>Review and send…</button>
      <button className="btn btn-sm" onClick={() => void notes.clear()}>Discard notes</button>
    </div>}
    {preview && <div className="grid gap-2 border-b border-line bg-chrome p-2 text-xs">
      <b>Message sent to the agent</b>
      <pre className="max-h-48 overflow-auto rounded border border-line bg-paper p-2 font-mono text-[11.5px] whitespace-pre-wrap">{notesMessage(notes.value)}</pre>
      <div className="flex flex-wrap gap-2"><button className="btn btn-sm btn-primary" disabled={sending} onClick={() => void send()}>{sending ? "◷ Sending…" : `Send ${notes.value.length} note${notes.value.length === 1 ? "" : "s"}`}</button><button className="btn btn-sm" disabled={sending} onClick={() => setPreview(false)}>Cancel</button></div>
      {notes.value.map((n, i) => <div key={i} className="flex items-start gap-2"><span className="min-w-0 flex-1 truncate">{n.path}{n.line != null ? `:${n.line}` : ""} · {n.text}</span><button className="btn btn-sm" onClick={() => notes.set((all) => all.filter((_, j) => j !== i))}>Remove</button></div>)}
    </div>}
    {status.error && !committed && (/not a git repository/i.test(status.error.message) ? <p className="p-3 text-xs text-ink-3">This folder is not a Git repository.</p> : <p className="p-3 text-xs text-gh-closed">{status.error.message}</p>)}
    {status.isPending && <p className="p-3 text-xs">Reading Git status…</p>}
    {(status.data || history.data) && !files.length && <p className="p-3 text-xs text-ink-3">{committed ? "No net file changes in these commits." : main ? "No tracked changes compared with main." : "Working tree clean."}</p>}
    {!!files.length && <div className="flex min-h-0 flex-1">
      {(!phone || showList) && fileList}
      {(!phone || !showList) && <div className="flex min-w-0 flex-1 flex-col">
        <div className="flex flex-wrap items-center gap-1.5 border-b border-line p-1.5 text-xs">
          {phone && <button className="btn btn-sm" onClick={() => setShowList(true)}><Icon name="chev" size={12} className="rotate-90" />Files</button>}
          <span className="min-w-0 flex-1 truncate font-mono" title={selected}>{selected}</span>
          <label className="flex items-center gap-1"><input type="checkbox" checked={selectedViewed} disabled={!diff.data} onChange={toggleViewed} />Viewed</label>
          <button className={`btn btn-sm ${wrap ? "bg-select!" : ""}`} aria-pressed={wrap} onClick={() => setWrap(!wrap)}>Wrap</button>
          {phone && at >= 0 && at < files.length - 1 && <button className="btn btn-sm" onClick={() => open(files[at + 1].path)}>Next file<Icon name="chev" size={12} className="-rotate-90" /></button>}
        </div>
        {diff.error && <p className="p-2 text-xs text-gh-closed">{diff.error.message}</p>}
        {diff.isPending && <p className="p-2 text-xs">Loading diff…</p>}
        {diff.data?.truncated && <p className="p-2 text-xs text-ink-3">Large diff: showing its beginning and end.</p>}
        {diff.data && !text && <p className="p-2 text-xs text-ink-3">No {committed ? "committed" : main ? "changes compared with main" : staged ? "staged" : "working tree"} changes for this file.</p>}
        {!!text && picked === null && <p className="border-b border-line-2 px-2 py-1 text-[11.5px] text-ink-3">{phone ? "Tap" : "Click"} a line to write a note for the agent.</p>}
        <div ref={codeRef} className="min-h-0 flex-1 overflow-auto"><div style={{ height: lines.getTotalSize(), position: "relative", minWidth: "100%", width: wrap ? "100%" : "max-content" }}>{lines.getVirtualItems().map((r) => {
          const line = rows[r.index];
          const pickable = line.number !== null;
          return <div key={r.index} data-index={r.index} ref={wrap ? lines.measureElement : undefined} style={{ position: "absolute", top: 0, left: 0, minWidth: "100%", width: wrap ? "100%" : undefined, transform: `translateY(${r.start}px)` }}>
            <PatchRow line={line} wrap={wrap} noted={pickable && fileNotes.some((n) => n.line === line.number && n.code === line.text)} selected={picked === r.index} onPick={pickable ? () => setPicked(r.index) : null} />
          </div>;
        })}</div></div>
        {picked !== null && rows[picked] && <div className="grid gap-1.5 border-t border-line bg-chrome p-2 text-xs">
          <span className="truncate">Note on <span className="font-mono">{selected}:{rows[picked].number}</span> <span className="font-mono text-ink-3">{rows[picked].text.slice(0, 80)}</span></span>
          <textarea autoFocus aria-label="Note for the agent" className="field-input h-16 resize-y py-1.5" value={draft} onChange={(e) => setDraft(e.target.value)} placeholder="What should the agent change here?" />
          <div className="flex gap-2"><button className="btn btn-sm btn-primary" disabled={!draft.trim()} onClick={addNote}>Add note</button><button className="btn btn-sm" onClick={() => { setPicked(null); setDraft(""); }}>Cancel</button></div>
        </div>}
      </div>}
    </div>}
  </div>;
}
