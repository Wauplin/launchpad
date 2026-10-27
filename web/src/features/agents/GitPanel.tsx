// Changes view of a session (read-only Git) through the shared DiffReview: viewed markers and line notes for the agent.
// Viewed markers and unsent notes are review state on the daemon (`session-viewed:`, `session-notes:`), so a review
// started on the phone continues on the laptop. Notes go to the agent as one message the owner reads before sending.
// The phone shows the file list or one file at a time.

import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { api } from "@/lib/api";
import { useReviewState } from "@/lib/reviewState";
import { Icon } from "@/components/Icon";
import { toast, toastError } from "@/components/Toast";
import { ConfirmButton, Empty, ErrorState, FilterChip, FilterChips, Loading } from "@/components/ui";
import { DiffReview, type LineNote } from "@/features/github/DiffReview";
import type { SessionGitStatus as Status } from "@/gen/SessionGitStatus";
import type { SessionGitHistory as History } from "@/gen/SessionGitHistory";
import type { SessionGitDiff as Diff } from "@/gen/SessionGitDiff";

type Note = LineNote;

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

export function GitPanel({ sessionId, initialMain = false, canMessage = true }: { sessionId: string; initialMain?: boolean; canMessage?: boolean }) {
  const [path, setPath] = useState<string | null>(null);
  const [main, setMain] = useState(initialMain);
  const [mode, setMode] = useState<"auto" | "live" | "committed">("auto");
  const [commit, setCommit] = useState("");
  const history = useQuery({ queryKey: ["session-git-commits", sessionId, ""], queryFn: () => api.get<History | null>(`/api/sessions/${sessionId}/git-commits`), refetchInterval: 5000 });
  const commitHistory = useQuery({ queryKey: ["session-git-commits", sessionId, commit], queryFn: () => api.get<History | null>(`/api/sessions/${sessionId}/git-commits?commit=${commit}`), enabled: !!commit, refetchInterval: commit ? 5000 : false });
  const selectedHistory = commit ? commitHistory.data : history.data;
  const [staged, setStaged] = useState(false);
  const status = useQuery({ queryKey: ["session-git", sessionId], queryFn: () => api.get<Status>(`/api/sessions/${sessionId}/git`), refetchInterval: 5000 });
  const committed = mode === "committed" || (mode === "auto" && !status.isPending && !status.data?.files.length && !!history.data?.commits.length);
  const files = (committed ? selectedHistory?.files : main ? status.data?.main_files : status.data?.files) ?? [];
  const selected = files.find((f) => f.path === path)?.path ?? files[0]?.path;
  const diff = useQuery({ queryKey: ["session-git-diff", sessionId, selected, main, committed, commit, committed ? selectedHistory?.head : null], queryFn: () => api.get<Diff>(`/api/sessions/${sessionId}/git-diff?path=${encodeURIComponent(selected!)}&main=${main}&committed=${committed}${committed && commit ? `&commit=${commit}` : ""}`), enabled: !!selected, refetchInterval: 5000 });
  const viewed = useReviewState<Record<string, string>>(`session-viewed:${sessionId}`, {});
  const notes = useReviewState<Note[]>(`session-notes:${sessionId}`, []);
  const [preview, setPreview] = useState(false);
  const [sending, setSending] = useState(false);
  const text = (staged && !main && !committed ? diff.data?.staged : diff.data?.unstaged) ?? "";
  const scope = committed ? `committed:${selectedHistory?.base}:${selectedHistory?.head}` : main ? "main" : staged ? "staged" : "tree";
  const key = (p: string) => `${scope}:${p}`;
  // A file stays viewed only while its diff is unchanged; other files show the marker from their last view.
  const isViewed = (p: string) => p === selected && diff.data ? viewed.value[key(p)] === signature(text) : key(p) in viewed.value;
  const toggleViewed = (p: string) => {
    if (p !== selected || !diff.data) return;
    viewed.set((v) => {
      const next = { ...v };
      if (next[key(p)] === signature(text)) delete next[key(p)];
      else next[key(p)] = signature(text);
      return next;
    });
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
  const live = (s: boolean, m: boolean) => { setMode("live"); setStaged(s); setMain(m); };
  const notGit = status.error && /not a git repository/i.test(status.error.message);
  return <div className="flex min-h-0 flex-1 flex-col">
    {history.data && (committed || history.data.merged_into) && <div className="flex min-w-0 items-center gap-2 border-b border-line px-3 py-1.5 text-[12px]">
      {history.data.merged_into ? <span className="max-w-1/2 truncate" title={`Merged into ${history.data.merged_into}`}>Merged into {history.data.merged_into}</span> : <span>Commits</span>}
      {committed && <select aria-label="Commit to review" className="field-input min-w-0 flex-1" value={commit} onChange={(e) => { setCommit(e.target.value); setPath(null); }}>
        <option value="">All commits ({history.data.commits.length})</option>
        {history.data.commits.map((c) => <option key={c.id} value={c.id}>{c.id.slice(0, 8)} · {c.subject}</option>)}
      </select>}
    </div>}
    {commit && commitHistory.error && <ErrorState className="m-2" error={`${commitHistory.error.message}. Select All commits to refresh the selection.`} />}
    {history.error && <ErrorState className="m-2" error={`Committed changes unavailable: ${history.error.message}`} />}
    {notes.value.length > 0 && <div className="flex flex-wrap items-center gap-2 border-b border-attn-line bg-attn-soft px-3 py-1.5 text-[12px]">
      <b className="text-attn">{notes.value.length} note{notes.value.length === 1 ? "" : "s"} for the agent</b>
      <button className="btn btn-sm btn-primary" disabled={!canMessage} title={canMessage ? "Read the exact message, then send it" : "The agent cannot take a message now"} onClick={() => setPreview(true)}>Review and send…</button>
      <ConfirmButton consequence="Deletes the unsent notes." confirmLabel="Discard notes" onConfirm={() => notes.clear()}>Discard notes</ConfirmButton>
    </div>}
    {preview && <div className="grid gap-2 border-b border-line bg-chrome p-2 text-[12px]">
      <b>Message sent to the agent</b>
      <pre className="max-h-48 overflow-auto rounded border border-line bg-paper p-2 font-mono text-[12px] whitespace-pre-wrap">{notesMessage(notes.value)}</pre>
      <div className="flex flex-wrap gap-2"><button className="btn btn-sm btn-primary" disabled={sending} onClick={() => void send()}>{sending ? "Sending…" : `Send ${notes.value.length} note${notes.value.length === 1 ? "" : "s"}`}</button><button className="btn btn-sm" disabled={sending} onClick={() => setPreview(false)}>Cancel</button></div>
      {notes.value.map((n, i) => <div key={i} className="flex items-start gap-2"><span className="min-w-0 flex-1 truncate">{n.path}{n.line != null ? `:${n.line}` : ""} · {n.text}</span><button className="btn btn-sm" onClick={() => notes.set((all) => all.filter((_, j) => j !== i))}>Remove</button></div>)}
    </div>}
    {status.error && !committed && (notGit ? <Empty icon="info">This folder is not a Git repository.</Empty> : <ErrorState className="m-2" error={status.error} />)}
    {status.isPending && <Loading what="Git status" />}
    {!notGit && !status.isPending && <DiffReview
      files={files.map((f) => ({ path: f.path, previous: f.previous, status: f.status }))}
      selected={selected}
      onSelect={setPath}
      patch={diff.data ? text : undefined}
      loading={!!selected && diff.isPending}
      error={diff.error}
      truncated={diff.data?.truncated}
      emptyPatch={`No ${committed ? "committed" : main ? "changes compared with main" : staged ? "staged" : "working tree"} changes for this file.`}
      viewed={isViewed}
      onToggleViewed={diff.data ? toggleViewed : undefined}
      notes={notes.value}
      onAddNote={(n) => notes.set((all) => [...all, n])}
      toolbar={<>
        <FilterChips label="Compared changes">
          <FilterChip active={!staged && !main && !committed} onClick={() => live(false, false)}>Working tree</FilterChip>
          <FilterChip active={staged && !main && !committed} onClick={() => live(true, false)}>Staged</FilterChip>
          <FilterChip active={main && !committed} disabled={!status.data?.main_changes} onClick={() => live(false, true)}>Compared with main</FilterChip>
          {!!history.data?.commits.length && <FilterChip active={committed} onClick={() => setMode("committed")}>Committed</FilterChip>}
        </FilterChips>
        <button className="iconbtn" aria-label="Refresh changes" title="Refresh changes" onClick={() => { void status.refetch(); void history.refetch(); if (commit) void commitHistory.refetch(); if (selected) void diff.refetch(); }}><Icon name="refresh" size={13} /></button>
      </>}
      banner={(status.data || history.data) && !files.length ? <Empty icon="check">{committed ? "No net file changes in these commits." : main ? "No tracked changes compared with main." : "Working tree clean."}</Empty> : undefined}
    />}
  </div>;
}
