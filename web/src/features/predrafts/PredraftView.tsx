import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useParams, useNavigate } from "@tanstack/react-router";
import { memo, useEffect, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { openGitHub } from "@/features/github/lib";
import { api } from "@/lib/api";
import type { Predraft } from "@/gen/Predraft";
import type { Note } from "@/gen/Note";
import type { Diff } from "@/features/github/types";
import { FilesChanged } from "@/features/github/FilesChanged";
import { Markdown } from "@/components/Markdown";
import { useComposer } from "@/stores/composer";
import { useLayout } from "@/stores/layout";
type Detail = Diff & { predraft: Predraft; status: string | null; commits: string[]; base_ahead: number; dirty: boolean; default_remote_branch: string };
export function PredraftView() {
  const { id } = useParams({ strict: false }) as { id: string };
  return <Review key={id} id={id} />;
}
function Review({ id }: { id: string }) {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const show = useLayout((s) => s.show);
  const q = useQuery({ queryKey: ["predrafts", id], queryFn: () => api.get<Detail>(`/api/predrafts/${id}`) });
  const p = q.data?.predraft;
  const [editing, setEditing] = useState(false);
  const [title, setTitle] = useState("");
  const [body, setBody] = useState("");
  const [notes, setNotes] = useState<Note[]>([]);
  const [notePath, setNotePath] = useState("");
  const [note, setNote] = useState("");
  const [tab, setTab] = useState("description");
  const [remote, setRemote] = useState("");
  const [draft, setDraft] = useState(true);
  const [confirm, setConfirm] = useState<"publish" | "discard" | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [loaded, setLoaded] = useState(false);
  const draftKey = `predraft-draft:${id}`;
  useEffect(() => {
    if (!p || loaded) return;
    let saved: { title?: string; body?: string; notes?: Note[]; note?: string; notePath?: string } = {};
    try { saved = JSON.parse(localStorage.getItem(draftKey) ?? "{}"); } catch { /* Start from daemon data. */ }
    setTitle(saved.title ?? p.title); setBody(saved.body ?? p.body); setNotes(saved.notes ?? p.notes);
    setNote(saved.note ?? ""); setNotePath(saved.notePath ?? "");
    setRemote(q.data?.default_remote_branch ?? ""); setLoaded(true);
    setEditing(saved.title !== undefined || saved.body !== undefined);
  }, [p, loaded]);
  useEffect(() => { if (loaded) localStorage.setItem(draftKey, JSON.stringify({ title: editing ? title : undefined, body: editing ? body : undefined, notes, note, notePath })); }, [loaded, title, body, notes, note, notePath, editing]);
  async function run(action: () => Promise<unknown>) {
    setBusy(true); setError("");
    try { await action(); setConfirm(null); await qc.invalidateQueries({ queryKey: ["predrafts"] }); }
    catch (e) { setError(String(e)); }
    finally { setBusy(false); }
  }
  function allNotes() { return note.trim() ? [...notes, { path: notePath || null, text: note.trim() }] : notes; }
  async function save() {
    const next = allNotes();
    await api.patch(`/api/predrafts/${id}`, { title: editing ? title : undefined, body: editing ? body : undefined, notes: next });
    setNotes(next); setNote(""); setEditing(false);
  }
  if (q.error) return <p className="p-6 text-gh-closed">{String(q.error)}</p>;
  if (!p || !q.data) return <p className="p-6">Loading local PR…</p>;
  const d = q.data;
  const active = !p.archived && !p.published_url;
  const running = ["starting", "working", "background", "in_terminal"].includes(d.status ?? "");
  const state = p.archived ? "Discarded" : p.published_url ? "Published" : running ? "In progress" : d.status === "waiting" ? "Ready for review" : d.status === "error" || d.status === "interrupted" ? "Agent needs attention" : "Local review";
  return <div className="flex h-full min-h-0 flex-col">
    <div className="border-b border-line bg-[#eaf8f5] p-4">
      <div className="flex flex-wrap items-center gap-2">
        <Link to="/projects/$id" params={{ id: p.project_id }} className="btn btn-sm no-underline">Project</Link>
        <span className="rounded bg-teal-soft px-2 py-1 text-xs font-semibold text-teal-ink">Pre-draft PR · {state}</span>
        <span className="font-mono text-xs text-ink-3">{p.branch} → {p.base_branch}</span>
        <button className="btn btn-sm ml-auto" disabled={busy} onClick={() => void run(async () => { qc.setQueryData(["predrafts", id], await api.post<Detail>(`/api/predrafts/${id}/refresh`)); })}>Refresh</button>
        {active && !p.pushed_head && !p.session_id && <button className="btn btn-sm" disabled={busy} onClick={() => { useComposer.getState().start({ projectId: p.project_id }); useComposer.getState().update((s) => ({ ...s, target: { kind: "existing_predraft", predraft_id: p.id }, text: `Continue this local PR: ${p.title}` })); }}>Launch agent</button>}
        {p.session_id && <button className="btn btn-sm" onClick={() => { show(p.session_id!); void navigate({ to: "/agents" }); }}>Agent session</button>}
        {!p.archived && <Link className="btn btn-sm no-underline" to="/projects/$id/code" params={{ id: p.project_id }} search={{ root: p.root }}>Code</Link>}
      </div>
      <h2 className="my-2 text-lg font-semibold">{p.title}</h2>
      {p.published_url && <button className="text-teal-ink hover:underline" onClick={() => openGitHub(p.published_url!)}>Published on GitHub #{p.published_number}</button>}
      {d.base_ahead > 0 && <p className="my-2 text-xs text-amber-800">Base moved: {p.base_branch} is {d.base_ahead} commits ahead of this branch. Review against the refreshed base before publishing.</p>}
      {d.dirty && <p className="my-2 text-xs text-amber-800">Uncommitted changes are excluded from this review. Ask the agent to commit them before publishing.</p>}
      {error && <p className="my-2 text-gh-closed">{error}</p>}
      <div className="flex gap-2">{["description", "files", "commits", "notes"].map((t) => <button key={t} className={`btn btn-sm ${tab === t ? "bg-select!" : ""}`} onClick={() => setTab(t)}>{t === "files" ? `Files changed (${d.files.length})` : t[0].toUpperCase() + t.slice(1)}</button>)}</div>
    </div>
    {tab === "files" && !p.archived && d.head ? <FilesChanged repo="local" number={0} head={d.head} local={{ id, root: p.root }} /> : <div className="min-h-0 flex-1 overflow-auto p-4">
      {tab === "description" && <>{editing && active ? <div className="grid gap-3"><input aria-label="PR title" className="field-input" value={title} onChange={(e) => setTitle(e.target.value)} /><textarea aria-label="PR description" className="field-input min-h-48" value={body} onChange={(e) => setBody(e.target.value)} /><button className="btn w-fit" disabled={busy} onClick={() => void run(save)}>Save title and description</button></div> : <><Markdown text={p.body || "No description yet. The agent can set it with launchpad pr update."} />{active && <button className="btn mt-3" onClick={() => { setTitle(p.title); setBody(p.body); setEditing(true); }}>Edit title and description</button>}</>}</>}
      {tab === "commits" && <Commits commits={d.commits} />}
      {tab === "notes" && <div className="grid max-w-[800px] gap-3">
        {notes.map((n, i) => <div key={i} className="rounded border border-line p-3"><b className="text-xs">{n.path ?? "General"}</b><textarea aria-label={`Review note ${i + 1}`} className="field-input mt-2 w-full" disabled={!active || busy} value={n.text} onChange={(e) => setNotes((a) => a.map((v, j) => j === i ? { ...v, text: e.target.value } : v))} />{active && <button className="btn btn-sm mt-2" disabled={busy} onClick={() => setNotes((a) => a.filter((_, j) => i !== j))}>Remove note</button>}</div>)}
        {active && <><select aria-label="Note file" className="field-input" value={notePath} onChange={(e) => setNotePath(e.target.value)}><option value="">General review note</option>{d.files.map((f) => <option key={f.filename} value={f.filename}>{f.filename}</option>)}</select><textarea aria-label="New review note" placeholder="Feedback for the agent…" className="field-input min-h-24" value={note} onChange={(e) => setNote(e.target.value)} /><div className="flex gap-2"><button className="btn" disabled={busy} onClick={() => void run(save)}>Save notes</button><button className="btn btn-primary" disabled={busy || running || !p.session_id || !allNotes().length} onClick={() => void run(async () => { await save(); await api.post(`/api/predrafts/${id}/feedback`); setNotes([]); })}>Send feedback to agent</button></div></>}
      </div>}
    </div>}
    {active && <div className="flex flex-wrap items-center gap-2 border-t border-line p-3">
      <input aria-label="Remote branch name" className="field-input min-w-48 flex-1" value={remote} onChange={(e) => { setRemote(e.target.value); setConfirm(null); }} placeholder="Remote branch name" />
      <label className="flex items-center gap-2 text-xs"><input type="checkbox" checked={draft} onChange={(e) => { setDraft(e.target.checked); setConfirm(null); }} />Publish as draft</label>
      <button className="btn btn-primary" disabled={busy || running || d.dirty || !d.head || !remote.trim()} onClick={() => { if (confirm !== "publish") { setConfirm("publish"); return; } void run(async () => { await save(); await api.post(`/api/predrafts/${id}/publish`, { remote_branch: remote, draft, head: d.head }); localStorage.removeItem(draftKey); }); }}>{busy ? "Working…" : confirm === "publish" ? "Confirm push and create GitHub PR" : "Publish"}</button>
      <button className="btn text-gh-closed" disabled={busy || running || d.dirty} onClick={() => { if (confirm !== "discard") { setConfirm("discard"); return; } void run(() => api.post(`/api/predrafts/${id}/discard`)); }}>{confirm === "discard" ? "Confirm delete branch and worktree" : "Discard"}</button>
      {confirm && <button className="btn btn-sm" onClick={() => setConfirm(null)}>Cancel</button>}
    </div>}
  </div>;
}

const CommitRow = memo(function CommitRow({ text }: { text: string }) { return <div className="flex h-8 items-center truncate border-b border-line-2 px-2 font-mono text-xs" title={text}>{text}</div>; });
function Commits({ commits }: { commits: string[] }) {
  const ref = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({ count: commits.length, getScrollElement: () => ref.current, estimateSize: () => 32, overscan: 4, useFlushSync: false });
  return <div ref={ref} className="h-[400px] overflow-auto"><div className="relative" style={{ height: v.getTotalSize() }}>{v.getVirtualItems().map((r) => <div key={r.key} className="absolute w-full" style={{ top: r.start }}><CommitRow text={commits[r.index]} /></div>)}</div>{!commits.length && <p className="text-ink-3">No commits yet.</p>}</div>;
}
