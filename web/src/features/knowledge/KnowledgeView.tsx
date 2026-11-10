// Knowledge: the knowledge base of a workspace, the only memory agents have. Notes (read, edit, concurrent
// edits, backlinks), Changes (who changed what, with the diff and Restore previous) and the two one-click
// helpers: the gardener (a normal launch, weekly with Repeat on a schedule) and the import of a runner's
// harness memories. Home stores everything; every change is restorable for 30 days.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate, useSearch } from "@tanstack/react-router";
import { memo, useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Icon } from "@/components/Icon";
import { Markdown } from "@/components/Markdown";
import { toast, toastError } from "@/components/Toast";
import { Badge, ConfirmButton, Empty, ErrorState, Choice, Choices, FilterChip, Loading, PageHeader, Tabs } from "@/components/ui";
import { PatchView } from "@/features/github/DiffReview";
import type { Change } from "@/gen/Change";
import type { Hit } from "@/gen/Hit";
import type { NativeMemory } from "@/gen/NativeMemory";
import type { Notes } from "@/gen/Notes";
import { api, ApiError } from "@/lib/api";
import { daemonUrl } from "@/lib/connection";
import { ago } from "@/lib/format";
import { usePhone } from "@/lib/phone";
import { useSessions } from "@/lib/queries";
import { useRunners } from "@/lib/runners";
import { useScope } from "@/lib/workspace";
import { PhoneHeader } from "@/mobile/PhoneHeader";
import { phoneBack, useBackClose } from "@/mobile/back";
import { useKeptDraft } from "@/lib/keptDraft";
import { useComposer } from "@/stores/composer";

const ROW = 30;
const CHANGE_ROW = 52;
const MEDIA = /\.(png|jpe?g|gif|webp|svg)$/i;
const VIDEO = /\.(mp4|webm)$/i;
const AUDIO = /\.(mp3|wav|ogg)$/i;

const fileUrl = (ws: string, q: string) => daemonUrl(`/api/knowledge/${ws}/file?${q}`);

async function getText(ws: string, q: string): Promise<string> {
  const res = await fetch(fileUrl(ws, q), { credentials: "include" });
  if (!res.ok) throw new ApiError(res.status, (await res.text().catch(() => "")) || res.statusText);
  return res.text();
}

const stem = (path: string) => path.split("/").pop()!.replace(/\.md$/i, "");

/** Front matter (shown as tokens) and the body. */
function splitFrontMatter(text: string): { meta: [string, string][]; body: string } {
  const m = /^---\n([\s\S]*?)\n---\n?/.exec(text);
  if (!m) return { meta: [], body: text };
  const meta = m[1].split("\n").filter((l) => /^[A-Za-z_]+:\s*\S/.test(l)).map((l) => { const i = l.indexOf(":"); return [l.slice(0, i), l.slice(i + 1).trim()] as [string, string]; });
  return { meta, body: text.slice(m[0].length) };
}

/** `[[name]]` and `[[name|label]]` become links the page resolves to notes. */
const withWikilinks = (text: string) => text.replace(/\[\[([^\]|\n]+)(?:\|([^\]\n]+))?\]\]/g, (_, name: string, label?: string) => `[${(label ?? name).trim()}](kb:${encodeURIComponent(name.trim())})`);

function resolveLink(paths: string[], name: string, from: string): string | null {
  const n = name.replace(/\.md$/i, "").toLowerCase();
  if (paths.includes(name)) return name;
  const dir = from.includes("/") ? from.slice(0, from.lastIndexOf("/") + 1) : "";
  if (paths.includes(dir + name)) return dir + name;
  return paths.find((p) => stem(p).toLowerCase() === n || p.toLowerCase() === n || p.toLowerCase() === `${n}.md`) ?? null;
}

export function KnowledgeView() {
  const phone = usePhone();
  const qc = useQueryClient();
  const scope = useScope();
  const ws = scope.workspace.id || null;
  const navigate = useNavigate();
  const [tab, setTab] = useState<"notes" | "changes">("notes");
  const [selected, setSelected] = useState<string | null>(null);
  const [historyOf, setHistoryOf] = useState<string | null>(null);
  const [panel, setPanel] = useState<"note" | "new" | "import">("note");
  const notes = useQuery({ queryKey: ["knowledge", ws, "notes"], queryFn: () => api.get<Notes>(`/api/knowledge/${ws}`), enabled: !!ws });
  const changes = useQuery({ queryKey: ["knowledge", ws, "changes"], queryFn: () => api.get<{ changes: Change[] }>(`/api/knowledge/${ws}/changes`), enabled: !!ws });
  const unseen = (changes.data?.changes ?? []).filter((c) => c.id > (notes.data?.seen ?? 0) && c.author?.kind === "agent").length;
  // A link to one note (`/knowledge?note=<path>`, from a bot chat's Changed line) opens it.
  const linked = (useSearch({ strict: false }) as { note?: string }).note;
  useEffect(() => { setSelected(linked ?? null); setHistoryOf(null); setPanel("note"); if (linked) setTab("notes"); }, [ws, linked]);
  useEffect(() => {
    if (!selected && notes.data && !phone) setSelected(notes.data.files.some((f) => f.path === "INDEX.md") ? "INDEX.md" : notes.data.files[0]?.path ?? null);
  }, [notes.data, selected, phone]);

  useBackClose(phone && panel !== "note", () => setPanel("note"));
  if (!ws) return <div className="flex h-full flex-col"><PageHeader title="Knowledge" /><Loading what="the knowledge base" /></div>;
  const refresh = () => qc.invalidateQueries({ queryKey: ["knowledge"] });
  const gardener = () => {
    if (!notes.data) return;
    const c = useComposer.getState();
    c.start();
    c.update((s) => ({ ...s, target: { kind: "temp" }, worktree: null, text: notes.data!.gardener_prompt, workspace_id: ws }));
    toast("The gardener is ready in the composer: pick a profile with a cheap model, and Repeat on a schedule to run it every week.");
  };
  const actions = <>
    <button className="btn btn-sm" title="Agents' memory files on a runner (Claude Code, Codex), imported once as your notes" onClick={() => { setTab("notes"); setPanel("import"); }}><Icon name="download" size={12} />Import memories</button>
    <button className="btn btn-sm" title="An agent that merges duplicates, adds links and keeps INDEX.md current" onClick={gardener}><Icon name="spark" size={12} />Run the gardener</button>
    <button className="btn btn-sm btn-primary" onClick={() => { setTab("notes"); setPanel("new"); }}><Icon name="plus" size={12} />New note</button>
  </>;
  const tabs = <Tabs label="Knowledge views" value={tab} onChange={(t) => { setTab(t); if (t === "notes") setHistoryOf(null); }} items={[{ id: "notes", label: "Notes", count: notes.data?.files.length }, { id: "changes", label: "Changes", count: unseen || undefined }]} tour="knowledge.tabs" />;
  const last = changes.data?.changes[0]?.at;
  const meta = last && <span>Last change {ago(last)}</span>;
  // The phone keeps the open note in the address (`?note=`, a history entry: back returns to the list); New note
  // and Import are overlays that back closes.
  const openNote = (path: string) => { setTab("notes"); setPanel("note"); if (phone) void navigate({ to: "/knowledge", search: { note: path } as never }); else setSelected(path); };
  const showHistory = (path: string) => { setHistoryOf(path); setTab("changes"); };
  return (
    <div className="flex h-full min-h-0 flex-col" data-tour="knowledge.page">
      {phone ? <PhoneHeader title="Knowledge" onBack={selected && tab === "notes" ? () => phoneBack("/knowledge") : undefined} actions={actions} /> : <PageHeader title="Knowledge" meta={meta} intro="The only memory agents have. Every change is restorable for 30 days." actions={actions} tabs={tabs} />}
      {phone && <div className="px-3">{tabs}</div>}
      {notes.isLoading ? <Loading what="the knowledge base" /> : notes.error ? <ErrorState error={notes.error} onRetry={() => void notes.refetch()} /> : notes.data && (
        tab === "notes"
          ? <NotesTab ws={ws} notes={notes.data} changes={changes.data?.changes ?? []} selected={selected} select={openNote} panel={panel} setPanel={setPanel} showHistory={showHistory} refresh={refresh} />
          : <ChangesTab ws={ws} notes={notes.data} changes={changes.data?.changes ?? []} loading={changes.isLoading} error={changes.error} historyOf={historyOf} clearHistory={() => setHistoryOf(null)} openNote={openNote} refresh={refresh} />
      )}
    </div>
  );
}

// ---------------------------------------------------------------- Notes

function NotesTab({ ws, notes, changes, selected, select, panel, setPanel, showHistory, refresh }: { ws: string; notes: Notes; changes: Change[]; selected: string | null; select: (p: string) => void; panel: "note" | "new" | "import"; setPanel: (p: "note" | "new" | "import") => void; showHistory: (p: string) => void; refresh: () => unknown }) {
  const phone = usePhone();
  const [filter, setFilter] = useState("");
  const words = filter.trim();
  const hits = useQuery({ queryKey: ["knowledge", ws, "search", words], queryFn: () => api.get<{ hits: Hit[] }>(`/api/knowledge/${ws}/search?q=${encodeURIComponent(words)}`), enabled: words.length >= 2, staleTime: 10_000 });
  const rows = useMemo(() => {
    if (!words) return notes.files.map((f) => ({ path: f.path, hit: null as Hit | null }));
    const byName = notes.files.filter((f) => f.path.toLowerCase().includes(words.toLowerCase())).map((f) => ({ path: f.path, hit: null as Hit | null }));
    const seen = new Set(byName.map((r) => r.path));
    const byText = (hits.data?.hits ?? []).filter((h) => !seen.has(h.path) && (seen.add(h.path), true)).map((h) => ({ path: h.path, hit: h }));
    return [...byName, ...byText];
  }, [notes.files, words, hits.data]);
  const scrollRef = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({ count: rows.length, getScrollElement: () => scrollRef.current, estimateSize: () => ROW, overscan: 12, useFlushSync: false });
  const showList = !phone || (!selected && panel === "note");
  const showPane = !phone || !!selected || panel !== "note";
  return (
    <div className="flex min-h-0 flex-1">
      {showList && <aside className={`flex min-h-0 flex-col border-r border-line ${phone ? "flex-1" : "w-[300px] flex-none"}`} data-tour="knowledge.notes">
        <div className="border-b border-line-2 p-2">
          <input className="field-input w-full" placeholder="Find notes by name or text" aria-label="Find notes" value={filter} onChange={(e) => setFilter(e.target.value)} />
        </div>
        {notes.concurrent.length > 0 && <div className="border-b border-line-2 bg-attn-soft px-3 py-2 text-[12px] text-attn">
          {notes.concurrent.length === 1 ? "1 concurrent edit waits" : `${notes.concurrent.length} concurrent edits wait`} for a merge: {notes.concurrent.map((c, i) => <span key={c.path}>{i > 0 && ", "}<button className="underline" onClick={() => select(c.note)}>{c.note}</button></span>)}
        </div>}
        {rows.length === 0 ? <Empty icon="search">{words ? (hits.isFetching ? "Searching…" : "No note matches.") : "No note yet."}</Empty> : <div ref={scrollRef} className="min-h-0 flex-1 overflow-y-auto">
          <div className="relative" style={{ height: v.getTotalSize() }}>
            {v.getVirtualItems().map((r) => <div key={rows[r.index].path} className="absolute inset-x-0" style={{ top: r.start, height: ROW }}>
              <NoteRow path={rows[r.index].path} hit={rows[r.index].hit} active={rows[r.index].path === selected && panel === "note"} onClick={select} />
            </div>)}
          </div>
        </div>}
      </aside>}
      {showPane && <section className="flex min-h-0 min-w-0 flex-1 flex-col">
        {panel === "import" ? <ImportPanel ws={ws} existing={notes.files.map((f) => f.path)} done={() => { setPanel("note"); void refresh(); }} />
          : panel === "new" ? <NoteEditor ws={ws} isNew paths={notes.files.map((f) => f.path)} done={(path) => { setPanel("note"); if (path) select(path); void refresh(); }} />
          : selected ? <NotePane key={selected} ws={ws} notes={notes} changes={changes} path={selected} select={select} showHistory={showHistory} refresh={refresh} />
          : <Empty fill icon="book">Choose a note.</Empty>}
      </section>}
    </div>
  );
}

const NoteRow = memo(function NoteRow({ path, hit, active, onClick }: { path: string; hit: Hit | null; active: boolean; onClick: (p: string) => void }) {
  const i = path.lastIndexOf("/");
  return <button className={`flex h-full w-full min-w-0 items-center gap-1.5 px-3 text-left text-[13px] hover:bg-sunk ${active ? "bg-sunk font-semibold" : ""}`} title={hit ? `${path}:${hit.line} ${hit.text}` : path} onClick={() => onClick(path)}>
    <Icon name={MEDIA.test(path) || VIDEO.test(path) || AUDIO.test(path) ? "clip" : "file"} size={12} className="flex-none text-ink-3" />
    <span className="min-w-0 truncate">{i >= 0 && <span className="text-ink-3">{path.slice(0, i + 1)}</span>}{path.slice(i + 1)}</span>
    {hit && <span className="min-w-0 flex-1 truncate text-[11px] text-ink-3">{hit.text}</span>}
  </button>;
});

function Authors({ changes }: { changes: Change[] }) {
  const agents = new Set(changes.filter((c) => c.author?.kind === "agent").map((c) => c.author!.session_id));
  const mine = changes.filter((c) => c.author?.kind === "owner");
  const parts = [agents.size ? `${agents.size} ${agents.size === 1 ? "session" : "sessions"}` : null, mine.length ? `you (${ago(mine[0].at)})` : null].filter(Boolean);
  if (!parts.length) return null;
  return <span>Written by {parts.join(", ")}</span>;
}

function NotePane({ ws, notes, changes, path, select, showHistory, refresh }: { ws: string; notes: Notes; changes: Change[]; path: string; select: (p: string) => void; showHistory: (p: string) => void; refresh: () => unknown }) {
  const entry = notes.files.find((f) => f.path === path);
  const isText = !!entry && !MEDIA.test(path) && !VIDEO.test(path) && !AUDIO.test(path) && entry.size < 1 << 20;
  const text = useQuery({ queryKey: ["knowledge", ws, "file", path, entry?.sha256], queryFn: () => getText(ws, `path=${encodeURIComponent(path)}`), enabled: isText, staleTime: Infinity });
  const [editing, setEditing] = useState(false);
  const paths = useMemo(() => notes.files.map((f) => f.path), [notes.files]);
  const backlinks = useQuery({ queryKey: ["knowledge", ws, "search", `[[${stem(path)}`], queryFn: () => api.get<{ hits: Hit[] }>(`/api/knowledge/${ws}/search?q=${encodeURIComponent(`[[${stem(path)}`)}`), enabled: path.endsWith(".md"), staleTime: 10_000 });
  const linking = [...new Set((backlinks.data?.hits ?? []).map((h) => h.path).filter((p) => p !== path))];
  const mine = changes.filter((c) => c.path === path);
  const concurrent = notes.concurrent.filter((c) => c.note === path);
  const ofConcurrent = notes.concurrent.find((c) => c.path === path);
  const links = useMemo(() => ({
    image: (src: string) => /^[a-z]+:/i.test(src) ? null : fileUrl(ws, `path=${encodeURIComponent(resolveLink(paths, decodeURIComponent(src), path) ?? src.replace(/^\.\//, ""))}`),
    link: (href: string) => {
      const name = href.startsWith("kb:") ? decodeURIComponent(href.slice(3)) : !/^[a-z]+:|^#|^\//i.test(href) ? decodeURIComponent(href) : null;
      if (name === null) return false;
      const target = resolveLink(paths, name, path);
      if (target) select(target);
      else toast(`There is no note ${name} yet`);
      return true;
    },
  }), [ws, paths, path, select]);
  if (!entry) return <Empty fill icon="file">This note was removed. Find it in Changes to restore it.</Empty>;
  if (editing) return <NoteEditor ws={ws} path={path} base={entry.sha256} initial={text.data ?? ""} paths={paths} done={() => { setEditing(false); void refresh(); }} />;
  const remove = () => api.delete<{ lines: string[] }>(`/api/knowledge/${ws}/file?path=${encodeURIComponent(path)}`).then((r) => { toast(r.lines.join(" ")); return refresh(); }, toastError);
  const resolve = (p: string, useOther: boolean) => api.post<{ lines: string[] }>(`/api/knowledge/${ws}/resolve`, { path: p, use_other: useOther }).then((r) => { toast(r.lines.join(" ")); return refresh(); }, toastError);
  const fm = text.data != null ? splitFrontMatter(text.data) : null;
  const src = fileUrl(ws, `path=${encodeURIComponent(path)}`);
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex flex-wrap items-center gap-2 border-b border-line-2 px-4 py-2">
        <span className="min-w-0 flex-1 truncate font-mono text-[13px]" title={path}>{path}</span>
        {isText && <button className="btn btn-sm" onClick={() => setEditing(true)}><Icon name="pencil" size={12} />Edit</button>}
        <button className="btn btn-sm" onClick={() => showHistory(path)}><Icon name="clock" size={12} />History</button>
        <ConfirmButton icon="trash" consequence={`Moves ${path} to the trash; Restore works for 30 days.`} confirmLabel="Delete" onConfirm={remove}>Delete</ConfirmButton>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4">
        {concurrent.map((c) => <div key={c.path} className="mb-3 flex flex-wrap items-center gap-2 rounded-card border border-attn bg-attn-soft px-3 py-2 text-[12px]">
          <span className="min-w-0 flex-1">A second version of this note was written while it changed: <button className="underline" onClick={() => select(c.path)}>{c.path}</button>. Keep one, or ask the gardener to merge them.</span>
          <ConfirmButton tone="publish" consequence="Removes the second version; this note stays as it is." confirmLabel="Keep this note" onConfirm={() => resolve(c.path, false)}>Keep this note</ConfirmButton>
          <ConfirmButton tone="publish" consequence="Replaces this note with the second version. The current one stays restorable." confirmLabel="Use the other version" onConfirm={() => resolve(c.path, true)}>Use the other version</ConfirmButton>
        </div>)}
        {ofConcurrent && <div className="mb-3 rounded-card border border-attn bg-attn-soft px-3 py-2 text-[12px]">This is a second version of <button className="underline" onClick={() => select(ofConcurrent.note)}>{ofConcurrent.note}</button>, written while that note changed. Open the note to keep one of them.</div>}
        {MEDIA.test(path) ? <img src={src} alt={path} className="max-h-[70vh] max-w-full rounded-control border border-line-2" />
          : VIDEO.test(path) ? <video src={src} controls className="max-h-[70vh] max-w-full" />
          : AUDIO.test(path) ? <audio src={src} controls />
          : !isText ? <Empty icon="file">This file is not shown here ({Math.round(entry.size / 1024)} KB). <a className="text-teal-ink hover:underline" href={src} target="_blank" rel="noreferrer">Open it</a></Empty>
          : text.isLoading ? <Loading what="the note" />
          : text.error ? <ErrorState error={text.error} />
          : fm && <>
            {fm.meta.length > 0 && <div className="mb-3 flex flex-wrap gap-1.5">{fm.meta.map(([k, val]) => <Badge key={k} tone={k === "status" && val === "superseded" ? "attn" : "neutral"} title={k}>{k === "title" ? val : `${k}: ${val}`}</Badge>)}</div>}
            {path.endsWith(".md") ? <Markdown className="markdown-body" text={withWikilinks(fm.body)} links={links} /> : <pre className="m-0 whitespace-pre-wrap font-mono text-[12px]">{text.data}</pre>}
          </>}
      </div>
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 border-t border-line-2 px-5 py-2 text-[12px] text-ink-3">
        <Authors changes={mine} />
        {linking.length > 0 && <span>Linked from {linking.slice(0, 6).map((p, i) => <span key={p}>{i > 0 && ", "}<button className="text-teal-ink hover:underline" onClick={() => select(p)}>{stem(p)}</button></span>)}{linking.length > 6 && ` and ${linking.length - 6} more`}</span>}
      </div>
    </div>
  );
}

function NoteEditor({ ws, path: initialPath, base, initial, isNew, paths, done }: { ws: string; path?: string; base?: string; initial?: string; isNew?: boolean; paths: string[]; done: (path?: string) => void }) {
  const today = new Date().toISOString().slice(0, 10);
  // Typed text survives back and a closed page (kept per note and version, `useKeptDraft`) until Save or Cancel.
  const draftKey = isNew ? `kb.${ws}.new` : `kb.${ws}.${initialPath}.${base ?? ""}`;
  const pathDraft = useKeptDraft(isNew ? `${draftKey}.path` : null, initialPath ?? "topics/");
  const [path, setPath] = [pathDraft.text, pathDraft.setText];
  const draft = useKeptDraft(draftKey, initial ?? `---\ntitle: \ntype: fact\nstatus: current\nvalid_from: ${today}\n---\n\n`);
  const [text, setText] = [draft.text, draft.setText];
  const drop = () => { draft.clear(); pathDraft.clear(); };
  const [busy, setBusy] = useState(false);
  const clean = path.trim().replace(/^\/+/, "");
  const finalPath = isNew && clean && !/\.[a-z0-9]+$/i.test(clean) ? `${clean}.md` : clean;
  const taken = isNew && paths.includes(finalPath);
  const valid = !!finalPath && !finalPath.endsWith("/") && !finalPath.split("/").some((c) => !c || c === "." || c === "..");
  const save = async () => {
    setBusy(true);
    try {
      const r = await api.put<{ lines: string[] }>(`/api/knowledge/${ws}/file`, { path: finalPath, content: text, base: isNew ? null : base });
      toast(r.lines.join(" "));
      drop();
      done(finalPath);
    } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex flex-wrap items-center gap-2 border-b border-line-2 px-4 py-2">
        {isNew ? <input autoFocus className="field-input min-w-0 flex-1 font-mono text-[12px]" aria-label="Path of the new note" value={path} onChange={(e) => setPath(e.target.value)} />
          : <span className="min-w-0 flex-1 truncate font-mono text-[13px]">{path}</span>}
        <button className="btn btn-sm" onClick={() => { drop(); done(); }}>Cancel</button>
        <button className="btn btn-sm btn-primary" disabled={busy || !valid || taken} title={taken ? "A note with this path exists" : undefined} onClick={() => void save()}>{busy ? "Saving…" : "Save"}</button>
      </div>
      {draft.kept && <div className="px-4 pt-2 text-[12px] text-ink-3">Draft kept: your unsaved text from before.</div>}
      {isNew && <div className="px-4 pt-2 text-[12px] text-ink-3">{taken ? "A note with this path exists: open it and choose Edit." : "Folders: people/, projects/, topics/, decisions/, howto/, sources/. Link notes with [[name]]."}</div>}
      <textarea className="field-input m-3 min-h-0 flex-1 resize-none p-3 font-mono text-[13px] leading-5" aria-label="Note" value={text} onChange={(e) => setText(e.target.value)} spellCheck />
    </div>
  );
}

// ---------------------------------------------------------------- Import

function ImportPanel({ ws, existing, done }: { ws: string; existing: string[]; done: () => void }) {
  const runners = (useRunners().data ?? []).filter((r) => r.runner.state === "approved" && !r.runner.burst);
  const [runner, setRunner] = useState<string | null>(null);
  const chosen = runner ?? runners.find((r) => r.connected)?.runner.id ?? null;
  const from = runners.find((r) => r.runner.id === chosen)?.runner.name ?? "a runner";
  const q = useQuery({
    queryKey: ["knowledge-memories", chosen],
    queryFn: () => api.get<{ memories: NativeMemory[] }>(`/api/knowledge/memories?runner=${chosen}`),
    enabled: !!chosen,
    retry: false,
  });
  const [off, setOff] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const files = q.data?.memories ?? [];
  const picked = files.filter((f) => !off.has(f.path));
  const run = async () => {
    setBusy(true);
    try {
      const r = await api.post<{ lines: string[] }>(`/api/knowledge/${ws}/import`, { from, files: picked.map((f) => ({ path: f.path, content: f.content })) });
      toast(`${r.lines.join(" ")}. Run the gardener to fold them into your notes.`);
      done();
    } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex flex-wrap items-center gap-2 border-b border-line-2 px-4 py-2">
        <b className="min-w-0 flex-1 text-[13px]">Import harness memories</b>
        <button className="btn btn-sm" onClick={done}>Cancel</button>
        <button className="btn btn-sm btn-primary" disabled={busy || picked.length === 0} onClick={() => void run()}>{busy ? "Importing…" : `Import ${picked.length} ${picked.length === 1 ? "file" : "files"}`}</button>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto px-5 py-3 text-[13px]">
        <p className="m-0 mb-3 text-[12px] text-ink-3">Claude Code and Codex keep their own memory files on each machine. Launchpad turns that memory off in its sessions; import the files once into <span className="font-mono">imported/</span>, then run the gardener to fold them into your notes. Nothing is removed from the machine.</p>
        {runners.length === 0 ? <Empty icon="term">Connect a runner first: memories are read on the machine where your agents ran.</Empty>
          : <Choices label="Runner" className="mb-3">{runners.map((r) => <Choice key={r.runner.id} active={r.runner.id === chosen} disabled={!r.connected} title={r.connected ? undefined : "Offline"} onClick={() => setRunner(r.runner.id)}>{r.runner.name}</Choice>)}</Choices>}
        {q.isLoading ? <Loading what="memory files" /> : q.error ? <ErrorState error={q.error} /> : q.data && (files.length === 0 ? <Empty icon="book">No harness memory found on {from}.</Empty> : <div className="grid gap-1">
          {files.map((f) => <label key={f.path} className="flex min-w-0 items-start gap-2 rounded-control px-2 py-1 hover:bg-sunk">
            <input type="checkbox" className="mt-1" checked={!off.has(f.path)} onChange={(e) => setOff((s) => { const n = new Set(s); if (e.target.checked) n.delete(f.path); else n.add(f.path); return n; })} />
            <span className="grid min-w-0">
              <span className="truncate font-mono text-[12px]">{f.path}{existing.includes(f.path) && <span className="ml-2 font-sans text-attn">replaces the earlier import</span>}</span>
              <span className="truncate text-[11px] text-ink-3" title={f.source}>{f.source} · {f.content.split("\n").length} lines</span>
            </span>
          </label>)}
        </div>)}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------- Changes

function ChangesTab({ ws, notes, changes, loading, error, historyOf, clearHistory, openNote, refresh }: { ws: string; notes: Notes; changes: Change[]; loading: boolean; error: unknown; historyOf: string | null; clearHistory: () => void; openNote: (p: string) => void; refresh: () => unknown }) {
  const phone = usePhone();
  const rows = useMemo(() => historyOf ? changes.filter((c) => c.path === historyOf) : changes, [changes, historyOf]);
  const [open, setOpen] = useState<number | null>(null);
  const current = rows.find((c) => c.id === open) ?? (phone ? null : rows[0]) ?? null;
  const scrollRef = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({ count: rows.length, getScrollElement: () => scrollRef.current, estimateSize: () => CHANGE_ROW, overscan: 8, useFlushSync: false });
  const newest = changes[0]?.id ?? 0;
  const sessions = useSessions().data;
  const titles = useMemo(() => new Map((sessions ?? []).map((x) => [x.id, x.title])), [sessions]);
  const unseen = changes.filter((c) => c.id > notes.seen && c.author?.kind === "agent").length;
  const markSeen = () => api.post(`/api/knowledge/${ws}/seen`, { change: newest }).then(refresh, toastError);
  if (loading) return <Loading what="changes" />;
  if (error) return <ErrorState error={error} />;
  return (
    <div className="flex min-h-0 flex-1">
      {(!phone || !current) && <aside className={`flex min-h-0 flex-col border-r border-line ${phone ? "flex-1" : "w-[380px] flex-none"}`} data-tour="knowledge.changes">
        <div className="flex flex-wrap items-center gap-2 border-b border-line-2 px-3 py-2 text-[12px] text-ink-3">
          {historyOf ? <FilterChip active onClick={clearHistory}>{historyOf} <Icon name="x" size={12} /></FilterChip> : <span className="min-w-0 flex-1">{unseen ? `${unseen} new since your last look` : "Nothing new since your last look"}</span>}
          {historyOf && <span className="flex-1" />}
          {unseen > 0 && !historyOf && <button className="btn btn-sm" onClick={() => void markSeen()}><Icon name="check" size={12} />Mark all seen</button>}
        </div>
        {rows.length === 0 ? <Empty icon="clock">No change in the last 30 days.</Empty> : <div ref={scrollRef} className="min-h-0 flex-1 overflow-y-auto">
          <div className="relative" style={{ height: v.getTotalSize() }}>
            {v.getVirtualItems().map((r) => <div key={rows[r.index].id} className="absolute inset-x-0" style={{ top: r.start, height: CHANGE_ROW }}>
              <ChangeRow c={rows[r.index]} title={titles.get(rows[r.index].author?.session_id ?? "")} isNew={rows[r.index].id > notes.seen && rows[r.index].author?.kind === "agent"} active={rows[r.index].id === current?.id} onClick={setOpen} />
            </div>)}
          </div>
        </div>}
      </aside>}
      {current && <ChangeDetail key={current.id} ws={ws} c={current} exists={notes.files.some((f) => f.path === current.path)} back={phone ? () => setOpen(null) : undefined} openNote={openNote} refresh={refresh} />}
    </div>
  );
}

const what = (c: Change) => !c.sha256 ? "created" : !c.new_sha256 ? "removed" : "changed";

const ChangeRow = memo(function ChangeRow({ c, title, isNew, active, onClick }: { c: Change; title?: string; isNew: boolean; active: boolean; onClick: (id: number) => void }) {
  const a = c.author;
  return <button className={`grid h-full w-full min-w-0 content-center gap-0.5 border-b border-line-2 px-3 text-left hover:bg-sunk ${active ? "bg-sunk" : ""}`} onClick={() => onClick(c.id)}>
    <span className="flex min-w-0 items-center gap-1.5 text-[13px]">
      {isNew && <span className="size-1.5 flex-none rounded-full bg-teal" title="New since your last look" />}
      <span className="min-w-0 truncate font-mono text-[12px]">{c.path}</span>
      {c.added != null && c.added > 0 && <span className="text-[11px] text-gh-open tabular-nums">+{c.added}</span>}
      {c.removed != null && c.removed > 0 && <span className="text-[11px] text-danger tabular-nums">−{c.removed}</span>}
      {what(c) !== "changed" && <span className="text-[11px] text-ink-3">{what(c)}</span>}
      <span className="ml-auto flex-none text-[11px] text-ink-3">{ago(c.at)}</span>
    </span>
    <span className="truncate text-[11px] text-ink-3">{a ? `${a.kind === "agent" ? `Agent "${title ?? a.name}"` : a.name}${a.model ? ` · ${a.model}` : ""}${a.runner ? ` · ${a.runner}` : ""}` : "Unknown author"}</span>
  </button>;
});

function ChangeDetail({ ws, c, exists, back, openNote, refresh }: { ws: string; c: Change; exists: boolean; back?: () => void; openNote: (p: string) => void; refresh: () => unknown }) {
  const isText = !MEDIA.test(c.path) && !VIDEO.test(c.path) && !AUDIO.test(c.path);
  const q = useQuery({ queryKey: ["knowledge", ws, "diff", c.id], queryFn: () => api.get<{ patch: string }>(`/api/knowledge/${ws}/changes/${c.id}/diff`), enabled: isText, staleTime: Infinity });
  const restore = () => api.post<{ lines: string[] }>(`/api/knowledge/${ws}/restore`, { change: c.id }).then((r) => { toast(r.lines.join(" ")); return refresh(); }, toastError);
  const a = c.author;
  const kind = what(c);
  const title = useSessions().data?.find((x) => x.id === a?.session_id)?.title;
  return (
    <section className="flex min-h-0 min-w-0 flex-1 flex-col">
      <div className="flex flex-wrap items-center gap-2 border-b border-line-2 px-4 py-2">
        {back && <button className="iconbtn" aria-label="Back to changes" onClick={back}><Icon name="chev" size={16} className="rotate-90" /></button>}
        <span className="min-w-0 flex-1 truncate font-mono text-[13px]">{c.path} <span className="font-sans text-[12px] text-ink-3">· {kind} {ago(c.at)}</span></span>
        {exists && <button className="btn btn-sm" onClick={() => openNote(c.path)}><Icon name="file" size={12} />Open note</button>}
        <ConfirmButton consequence={kind === "created" ? `Removes ${c.path}, which this change created. It stays restorable.` : kind === "removed" ? `Puts ${c.path} back.` : `Puts back ${c.path} as it was before this change. The current version stays restorable.`} confirmLabel={kind === "created" ? "Undo" : "Restore"} onConfirm={restore}>{kind === "removed" ? "Restore" : kind === "created" ? "Undo" : "Restore previous"}</ConfirmButton>
      </div>
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 border-b border-line-2 px-4 py-1.5 text-[12px] text-ink-3">
        {a?.kind === "agent" ? <>
          <span>Agent <b className="text-ink-2">{title ?? a.name}</b></span>
          {a.model && <span>{a.model}</span>}
          {a.runner && <span>on {a.runner}</span>}
          {a.session_id && <Link to="/agents" search={{ session: a.session_id }} className="text-teal-ink hover:underline">Open the session</Link>}
        </> : <span>{a?.name ?? "Unknown author"}</span>}
      </div>
      <div className="min-h-0 flex-1 overflow-auto">
        {!isText ? <div className="p-4">{c.new_sha256 && <img src={fileUrl(ws, `sha256=${c.new_sha256}`)} alt={c.path} className="max-h-[60vh] max-w-full" />}</div>
          : q.isLoading ? <Loading what="the change" />
          : q.error ? <ErrorState error={q.error} />
          : q.data?.patch ? <PatchView patch={q.data.patch} path={c.path} language="markdown" split={false} wrap />
          : <Empty icon="file">No text difference to show.</Empty>}
      </div>
    </section>
  );
}
