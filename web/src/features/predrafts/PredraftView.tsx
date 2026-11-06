// A Local PR: a Launchpad branch on Home Git under review, published to GitHub or merged on Home only with the owner's click.
// Local PRs opened by one agent form a change set, published together.
// Description · Changes · Commits. Line notes from Changes and general notes go to the agent as feedback.

import { usePhone } from "@/lib/phone";
import { Icon } from "@/components/Icon";
import { PhoneHeader } from "@/mobile/PhoneHeader";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useParams, useNavigate } from "@tanstack/react-router";
import { memo, useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { openGitHub } from "@/features/github/lib";
import { api } from "@/lib/api";
import { useReviewState } from "@/lib/reviewState";
import type { Predraft } from "@/gen/Predraft";
import type { Note } from "@/gen/Note";
import type { Diff } from "@/features/github/types";
import { FilesChanged } from "@/features/github/FilesChanged";
import type { LineNote } from "@/features/github/DiffReview";
import { Markdown } from "@/components/Markdown";
import { toast } from "@/components/Toast";
import { Badge, ConfirmButton, Empty, ErrorState, Loading, PageHeader, Tabs, type BadgeTone } from "@/components/ui";
import { LocalSuggestions } from "@/features/suggestions/Suggestions";
import { useComposer } from "@/stores/composer";
import { useLayout } from "@/stores/layout";
import { runAction } from "@/lib/actions";
import { projectName, useProjects } from "@/lib/queries";
type Member = { id: string; title: string; folder: string; branch: string; published_url: string | null; merged: boolean };
type Detail = Diff & { predraft: Predraft; status: string | null; commits: string[]; base_ahead: number; default_remote_branch: string; project_missing?: boolean; change_set?: Member[] };
type View = "description" | "files" | "commits";
const RUNNING = ["starting", "working", "background", "in_terminal"];

/** Line notes are stored as Local PR notes ("Line 12: …"); this reads the line back for the markers in Changes. */
const lineNoteText = (n: LineNote) => `Line ${n.line}${n.side === "old" ? " (removed line)" : ""}${n.code.trim() ? ` \`${n.code.trim().slice(0, 120)}\`` : ""}: ${n.text}`;
function asLineNote(n: Note): LineNote | null {
  const m = /^Line (\d+)( \(removed line\))?/.exec(n.text);
  return n.path && m ? { path: n.path, line: Number(m[1]), side: m[2] ? "old" : "new", code: "", text: n.text } : null;
}

export function PredraftView() {
  const { id } = useParams({ strict: false }) as { id: string };
  return <Review key={id} id={id} />;
}
function Review({ id }: { id: string }) {
  const phone = usePhone();
  const qc = useQueryClient();
  const navigate = useNavigate();
  const show = useLayout((s) => s.show);
  const q = useQuery({ queryKey: ["predrafts", id], queryFn: () => api.get<Detail>(`/api/predrafts/${id}`) });
  const p = q.data?.predraft;
  const [editing, setEditing] = useState(false);
  const [title, setTitle] = useState("");
  const [body, setBody] = useState("");
  const [notes, setNotes] = useState<Note[]>([]);
  const [note, setNote] = useState("");
  const [tab, setTab] = useState<View>("description");
  const [remote, setRemote] = useState("");
  const [draft, setDraft] = useState(true);
  const [fork, setFork] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [loaded, setLoaded] = useState(false);
  const previousNotes = useRef<Note[] | null>(null);
  useEffect(() => {
    if (!p) return;
    const previous = previousNotes.current;
    previousNotes.current = p.notes;
    if (!previous || !loaded) return;
    const added = p.notes.filter((n) => !previous.some((old) => old.path === n.path && old.text === n.text));
    if (added.length) setNotes((current) => [...current, ...added.filter((n) => !current.some((old) => old.path === n.path && old.text === n.text))]);
  }, [p?.notes, loaded]);
  // Unsaved edits and notes live in the daemon so they follow the owner across devices.
  const saved = useReviewState<{ title?: string; body?: string; notes?: Note[]; note?: string; notePath?: string }>(`predraft:${id}`, {});
  function apply(v: typeof saved.value) {
    if (!p) return;
    setTitle(v.title ?? p.title); setBody(v.body ?? p.body); setNotes(v.notes ?? p.notes);
    setNote(v.note ?? "");
    setEditing(v.title !== undefined || v.body !== undefined);
  }
  useEffect(() => {
    if (!p || loaded || !saved.loaded) return;
    apply(saved.value);
    setRemote(p.remote_branch ?? q.data?.default_remote_branch ?? ""); setFork(!!p.push_repo); setLoaded(true);
  }, [p, loaded, saved.loaded]);
  // Another window or device changed the draft.
  useEffect(() => { if (loaded && saved.remote) apply(saved.value); }, [saved.remote]);
  useEffect(() => { if (loaded) saved.set({ title: editing ? title : undefined, body: editing ? body : undefined, notes, note }); }, [loaded, title, body, notes, note, editing]);
  const projects = useProjects().data;
  const lineNotes = useMemo(() => notes.map(asLineNote).filter((n): n is LineNote => !!n), [notes]);
  async function run(action: () => Promise<unknown>, done?: string) {
    setBusy(true); setError(null);
    try { await action(); if (done) toast(done); await qc.invalidateQueries({ queryKey: ["predrafts"] }); }
    catch (e) { setError(e); }
    finally { setBusy(false); }
  }
  function allNotes() { return note.trim() ? [...notes, { path: null, text: note.trim() }] : notes; }
  async function save() {
    const next = allNotes();
    await api.patch(`/api/predrafts/${id}`, { title: editing ? title : undefined, body: editing ? body : undefined, notes: next });
    setNotes(next); setNote(""); setEditing(false);
  }
  if (q.error) return <ErrorState className="m-4" error={q.error} onRetry={() => void q.refetch()} />;
  if (!p || !q.data) return <Loading what="the Local PR" />;
  const d = q.data;
  if (d.project_missing) return <Orphan p={p} running={RUNNING.includes(d.status ?? "")} onDiscarded={() => void qc.invalidateQueries({ queryKey: ["predrafts"] })} />;
  const active = !p.archived && !p.published_url && !p.merged;
  const running = RUNNING.includes(d.status ?? "");
  const [state, tone]: [string, BadgeTone] = p.archived ? ["Discarded", "done"] : p.published_url ? ["Published", "ok"] : p.merged ? ["Merged", "ok"] : running ? ["Agent working", "work"] : d.status === "waiting" ? ["Ready for review", "teal"] : d.status === "error" || d.status === "interrupted" ? ["Agent needs attention", "danger"] : ["Local review", "neutral"];
  const refresh = () => void run(async () => { qc.setQueryData(["predrafts", id], await api.post<Detail>(`/api/predrafts/${id}/refresh`)); });
  const all = allNotes();
  const sendNotes = () => run(async () => { await save(); await api.post(`/api/predrafts/${id}/feedback`); setNotes([]); }, "Notes sent to the agent");
  const canSend = !busy && !running && !!p.session_id && all.length > 0;
  const headerActions = <>
    {p.session_id && <button className="btn btn-sm" onClick={() => { show(p.session_id!); void navigate({ to: "/agents", search: phone ? { session: p.session_id! } : {} }); }}>Open agent</button>}
    {active && !p.pushed_head && !p.session_id && <button className="btn btn-sm" disabled={busy} onClick={() => { useComposer.getState().start({ projectId: p.project_id }); useComposer.getState().update((s) => ({ ...s, target: { kind: "existing_predraft", predraft_id: p.id }, text: `Continue this Local PR: ${p.title}` })); }}>Launch agent</button>}
  </>;
  const meta = <>
    <Badge tone={tone} icon="pr-local">Local PR · {state}</Badge>
    <Link to="/projects/$id" params={{ id: p.project_id }}>{projectName(projects, p.project_id) || "Folder"}</Link>
    <span className="font-mono">{p.branch} → {p.base_branch}</span>
    <button className="iconbtn" aria-label="Refresh" title="Read the branch, commits and changes again" disabled={busy} onClick={refresh}><Icon name="refresh" size={13} /></button>
  </>;
  const tabs = <Tabs<View> label="Local PR views" value={tab} onChange={setTab} items={[{ id: "description", label: "Description" }, { id: "files", label: "Changes", count: d.files.length }, { id: "commits", label: "Commits", count: d.commits.length }]} />;
  const notesBar = all.length > 0 && tab !== "description" && <div className="flex flex-wrap items-center gap-2 border-b border-attn-line bg-attn-soft px-3 py-1.5 text-[12px]">
    <b className="text-attn">{all.length} note{all.length === 1 ? "" : "s"} for the agent</b>
    <button className="btn btn-sm" onClick={() => setTab("description")}>Edit notes</button>
    <button className="btn btn-sm btn-primary ml-auto" disabled={!canSend} title={p.session_id ? undefined : "No agent works on this Local PR"} onClick={() => void sendNotes()}>Send to agent</button>
  </div>;
  return <div className="flex h-full min-h-0 flex-col">
    {phone ? <><PhoneHeader title={p.title} subtitle={`Local PR · ${state}`} onBack={() => void navigate({ to: "/", search: { filter: "review" } })} backLabel="Back to Today" actions={<>{headerActions}<button className="btn btn-sm" disabled={busy} onClick={refresh}>Refresh</button></>} details={<p className="break-all font-mono text-xs text-ink-3">{p.branch} → {p.base_branch}</p>} /><div className="px-3">{tabs}</div></>
      : <PageHeader title={p.title} meta={meta} actions={headerActions} tabs={tabs} />}
    {p.published_url && <div className="border-b border-line-2 px-4 py-2 text-[12px]"><button className="text-teal-ink hover:underline" onClick={() => openGitHub(p.published_url!)}>Published on GitHub as #{p.published_number}</button><span className="text-ink-3"> · New commits of agents on this branch reach the PR through Home.</span></div>}
    {p.merged && <div className="border-b border-line-2 px-4 py-2 text-[12px] text-ink-2">Merged into <span className="font-mono">{p.base_branch}</span> on Home at <span className="font-mono">{p.merged.head.slice(0, 8)}</span>.</div>}
    {active && d.base_ahead > 0 && <div className="grid gap-1 border-b border-attn-line bg-attn-soft px-4 py-2 text-[12px] text-attn">
      <span className="flex items-center gap-1.5"><Icon name="alert" size={12} />Base moved: {p.base_branch} is {d.base_ahead} commits ahead of this branch. Review against the refreshed base before publishing.</span>
    </div>}
    {!!d.change_set?.length && <ChangeSet members={d.change_set} active={active} busy={busy} onPublishAll={() => run(async () => { await save(); await api.post(`/api/predrafts/${id}/publish-set`, { draft }); await saved.clear(); }, "Change set published")} />}
    {p.remote_sync_note && <div className="border-b border-line px-3 py-2 text-[12px] text-ink-2">{p.remote_sync_note}</div>}
    {active && p.pushed_head && <div className="flex flex-wrap items-center gap-2 border-b border-attn-line bg-attn-soft px-4 py-2 text-[12px] text-attn">
      <Icon name="alert" size={12} /><span className="flex-1">Publication of <span className="font-mono">{p.pushed_head.slice(0, 8)}</span>{p.remote_branch ? <> to <span className="font-mono">{p.push_repo ?? "origin"}:{p.remote_branch}</span></> : null} did not finish. Retry Publish, or unlock it.</span>
      <ConfirmButton disabled={busy} consequence="Agents can change the branch again. A branch already pushed keeps its name, and the next Publish updates it. Nothing is deleted on GitHub." confirmLabel="Unlock" onConfirm={() => run(() => api.post(`/api/predrafts/${id}/unlock`), "Local PR unlocked")}>Unlock</ConfirmButton>
    </div>}
    {!!error && <ErrorState className="m-2" error={error} />}
    {notesBar}
    {tab === "files" && !p.archived && d.head ? <FilesChanged repo="local" number={0} head={d.head} local={{ id }} notes={lineNotes} onAddNote={active ? (n) => setNotes((all) => [...all, { path: n.path, text: lineNoteText(n) }]) : undefined} /> : <div className="min-h-0 flex-1 overflow-auto p-4">
      {tab === "files" && <Empty icon="info">{p.archived ? "This Local PR was discarded." : "No commits to compare yet."}</Empty>}
      {tab === "description" && <div className="grid max-w-[800px] gap-4">
        {editing && active ? <div className="grid gap-3"><input aria-label="PR title" className="field-input" value={title} onChange={(e) => setTitle(e.target.value)} /><textarea aria-label="PR description" className="field-input min-h-48" value={body} onChange={(e) => setBody(e.target.value)} /><div className="flex gap-2"><button className="btn btn-primary w-fit" disabled={busy} onClick={() => void run(save, "Title and description saved")}>Save title and description</button><button className="btn" disabled={busy} onClick={() => { setTitle(p.title); setBody(p.body); setEditing(false); }}>Cancel</button></div></div>
          : <div><Markdown text={p.body || "No description yet. The agent can set it with launchpad pr update."} />{active && <button className="btn btn-sm mt-3" onClick={() => { setTitle(p.title); setBody(p.body); setEditing(true); }}><Icon name="pencil" size={12} />Edit title and description</button>}</div>}
        {active && <section className="grid gap-2 rounded-lg border border-line p-3">
          <h3 className="seclabel m-0">Notes for the agent</h3>
          {notes.length === 0 && <p className="text-[12px] text-ink-3">Click a line in Changes to add a line note, or write a general note below.</p>}
          {notes.map((n, i) => <div key={i} className="grid gap-1"><span className="font-mono text-[12px] text-ink-3">{n.path ?? "General"}</span><div className="flex items-start gap-2"><textarea aria-label={`Note ${i + 1}`} className="field-input min-h-14 flex-1" disabled={busy} value={n.text} onChange={(e) => setNotes((a) => a.map((v, j) => j === i ? { ...v, text: e.target.value } : v))} /><button className="iconbtn" aria-label="Remove note" title="Remove note" disabled={busy} onClick={() => setNotes((a) => a.filter((_, j) => i !== j))}><Icon name="trash" size={13} /></button></div></div>)}
          <textarea aria-label="General note" placeholder="General note for the agent…" className="field-input min-h-16" value={note} onChange={(e) => setNote(e.target.value)} />
          <div className="flex flex-wrap items-center gap-2"><button className="btn btn-sm btn-primary" disabled={!canSend} title={p.session_id ? undefined : "No agent works on this Local PR"} onClick={() => void sendNotes()}>Send {all.length || ""} note{all.length === 1 ? "" : "s"} to agent</button><button className="btn btn-sm" disabled={busy} onClick={() => void run(save, "Notes saved")}>Save notes</button><span className="text-[12px] text-ink-3">Notes stay on Home.</span></div>
        </section>}
        <LocalSuggestions id={id} />
      </div>}
      {tab === "commits" && <Commits commits={d.commits} />}
    </div>}
    {active && <div data-tour="predraft.actions" className="phone-publish-bar flex flex-wrap items-center gap-2 border-t border-line p-3">
      <input aria-label="Remote branch name" className="field-input min-w-48 flex-1" value={remote} onChange={(e) => setRemote(e.target.value)} placeholder="Remote branch name" />
      <label className="flex items-center gap-2 text-[12px]"><input type="checkbox" checked={draft} onChange={(e) => setDraft(e.target.checked)} />As a GitHub draft</label>
      <label className="flex items-center gap-2 text-[12px]" title="Push to your fork of origin and open the PR from it, when you cannot push to origin"><input type="checkbox" checked={fork} disabled={!!p.pushed_head} onChange={(e) => setFork(e.target.checked)} />From my fork</label>
      <ConfirmButton tone="publish" small={false} className="btn-primary" disabled={busy || running || !d.head || !remote.trim()} consequence={`Home pushes the reviewed commit ${d.head?.slice(0, 8) ?? ""} as ${remote.trim()} to ${fork ? "your fork" : "origin"} and creates ${draft ? "a GitHub draft" : "a"} PR on GitHub.`} confirmLabel="Push and create PR" onConfirm={() => run(async () => { await save(); await runAction("local-pr.publish", { id }, { remote_branch: remote, draft, head: d.head, fork }); await saved.clear(); }, "Local PR published")}>Publish</ConfirmButton>
      <ConfirmButton tone="publish" small={false} disabled={busy || running || !d.head} consequence={`Home pushes the reviewed commit to ${p.base_branch} on the folder's remote, then fast-forwards ${p.base_branch} on Home; if the remote refuses the push, nothing is merged. If ${p.base_branch} moved, ask the agent to rebase first.`} confirmLabel={`Merge into ${p.base_branch}`} onConfirm={() => run(async () => { await runAction("local-pr.merge", { id }, { head: d.head }); await saved.clear(); }, `Merged into ${p.base_branch}`)}>Merge</ConfirmButton>
      <ConfirmButton small={false} disabled={busy || running} consequence="Deletes the branch on Home. Its tip stays restorable from the folder page for 30 days." confirmLabel="Discard" onConfirm={() => run(() => api.post(`/api/predrafts/${id}/discard`), "Local PR discarded")}>Discard</ConfirmButton>
    </div>}
  </div>;
}

/** The folder of this Local PR was removed from Launchpad: show the record and offer Discard only. */
function Orphan({ p, running, onDiscarded }: { p: Predraft; running: boolean; onDiscarded: () => void }) {
  const [error, setError] = useState<unknown>(null);
  const active = !p.archived && !p.published_url && !p.merged;
  return <div className="h-full overflow-auto p-6">
    <div className="max-w-[760px] rounded-lg border border-line bg-paper p-4">
      <Badge tone="attn" icon="alert">Local PR · folder removed</Badge>
      <h2 className="my-2 text-[18px] font-semibold">{p.title}</h2>
      <p className="font-mono text-[12px] text-ink-3">{p.branch} → {p.base_branch}</p>
      <p className="mt-3 text-[13px] text-ink-2">The folder of this Local PR is no longer registered in Launchpad, so its commits and changes cannot be reviewed or published from here. {active ? "Discard archives this record." : p.archived ? "This Local PR was discarded." : p.merged ? "This Local PR was merged." : "This Local PR was published."}</p>
      {p.published_url && <button className="text-teal-ink hover:underline" onClick={() => openGitHub(p.published_url!)}>Published on GitHub as #{p.published_number}</button>}
      {p.body && <div className="mt-3 border-t border-line pt-3"><Markdown text={p.body} /></div>}
      {p.remote_sync_note && <div className="border-b border-line px-3 py-2 text-[12px] text-ink-2">{p.remote_sync_note}</div>}
      {!!error && <ErrorState className="mt-2" error={error} />}
      {active && <div className="mt-4 flex items-center gap-2">
        <ConfirmButton small={false} disabled={running} consequence="Archives the record." confirmLabel="Discard" onConfirm={() => api.post(`/api/predrafts/${p.id}/discard`).then(onDiscarded, setError)}>Discard</ConfirmButton>
        {running && <span className="text-[12px] text-ink-3">An agent is still working on this branch.</span>}
      </div>}
    </div>
  </div>;
}

const CommitRow = memo(function CommitRow({ text }: { text: string }) { return <div className="flex h-7 items-center truncate border-b border-line-2 px-2 font-mono text-[12px]" title={text}>{text}</div>; });
function Commits({ commits }: { commits: string[] }) {
  const ref = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({ count: commits.length, getScrollElement: () => ref.current, estimateSize: () => 28, overscan: 4, useFlushSync: false });
  if (!commits.length) return <Empty icon="commit">No commits yet.</Empty>;
  return <div ref={ref} className="h-full max-h-[560px] overflow-auto"><div className="relative" style={{ height: v.getTotalSize() }}>{v.getVirtualItems().map((r) => <div key={r.key} className="absolute w-full" style={{ top: r.start }}><CommitRow text={commits[r.index]} /></div>)}</div></div>;
}

/** The other Local PRs opened by the same agent: reviewed here, published together with Publish all. */
function ChangeSet({ members, active, busy, onPublishAll }: { members: Member[]; active: boolean; busy: boolean; onPublishAll: () => void }) {
  const open = members.filter((m) => !m.published_url && !m.merged).length;
  return <div className="flex flex-wrap items-center gap-2 border-b border-line-2 px-4 py-2 text-[12px]">
    <span className="text-ink-3">Change set:</span>
    {members.map((m) => <Link key={m.id} to="/predrafts/$id" params={{ id: m.id }} className="lp-chip h-[22px]! px-2! text-[11px]!" title={m.branch}>{m.folder} · {m.title}{m.published_url ? " · published" : m.merged ? " · merged" : ""}</Link>)}
    {active && <ConfirmButton tone="publish" disabled={busy} className="ml-auto" consequence={`Publishes this Local PR and the ${open} other open one${open === 1 ? "" : "s"} of the set, each with its default remote branch, and links the pull requests to each other.`} confirmLabel="Publish all" onConfirm={onPublishAll}>Publish all</ConfirmButton>}
  </div>;
}
