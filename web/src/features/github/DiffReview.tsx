// One Changes view for every diff: PR files, Local PR files, session Changes (and commit diffs through FilePatch).
// File list with viewed markers and progress, Unified · Split, Wrap, syntax colors and line notes. Hosts own the
// data: where the patch comes from, what "viewed" means and where notes go (PR line comments posted with Submit
// review, notes for the agent elsewhere). Rows are virtualized; wrapped rows are measured.

import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Icon } from "@/components/Icon";
import { Empty, ErrorState, Choice, Choices, Loading } from "@/components/ui";
import { highlight, type Tokens } from "@/features/code/highlight";
import { isDemo } from "@/demo/mode";
import { storageAllowed } from "@/lib/clientStorage";
import { usePhone } from "@/lib/phone";
import { useBackClose } from "@/mobile/back";
import { useReviewState } from "@/lib/reviewState";
import { useTheme } from "@/stores/theme";

export type DiffEntry = { path: string; previous?: string | null; status: string; additions?: number | null; deletions?: number | null };
/** A note on one diff line. `side`: the old (deleted) or new file line. `head`: the PR head the note was written on. */
export type LineNote = { path: string; line: number | null; side?: "old" | "new"; code: string; text: string; head?: string; startLine?: number };

type Line = { text: string; kind: "hunk" | "add" | "del" | "context" | "note" | "meta"; old: number | null; next: number | null; index: number };

/** Parses a unified patch. Git headers before the first hunk (diff, index, ---, +++) become meta lines. */
export function parsePatch(patch: string): Line[] {
  let old = 0, next = 0, inHunk = false;
  return patch.split("\n").filter((s, i, a) => i < a.length - 1 || s !== "").map((text, index): Line => {
    const m = /^@@ -(\d+)(?:,\d+)? \+(\d+)/.exec(text);
    if (m) { old = Number(m[1]); next = Number(m[2]); inHunk = true; return { text, kind: "hunk", old: null, next: null, index }; }
    if (!inHunk || /^diff --git /.test(text)) { inHunk = false; return { text, kind: "meta", old: null, next: null, index }; }
    const kind = text.startsWith("+") ? "add" : text.startsWith("-") ? "del" : text.startsWith("\\") ? "note" : "context";
    return { text: kind === "note" ? text : text.slice(1), kind, old: kind === "add" || kind === "note" ? null : old++, next: kind === "del" || kind === "note" ? null : next++, index };
  });
}

const EXT: Record<string, string> = { rs: "rust", py: "python", ts: "typescript", tsx: "tsx", js: "javascript", jsx: "jsx", json: "json", md: "markdown", toml: "toml", yml: "yaml", yaml: "yaml", css: "css", html: "html", sh: "bash", c: "c", h: "c", cpp: "cpp", go: "go" };
export const langOf = (path: string) => EXT[path.split(".").pop() ?? ""] ?? "text";

const tone = (k: Line["kind"]) => k === "add" ? "bg-diff-add" : k === "del" ? "bg-diff-del" : k === "hunk" ? "bg-work-soft text-ink-3" : k === "meta" ? "text-ink-3" : "";
const noteLine = (l: Line) => ({ line: l.kind === "del" ? l.old : l.next, side: (l.kind === "del" ? "old" : "new") as "old" | "new" });

const Cell = memo(function Cell({ line, tokens, wrap, noted, picked, onPick, gutter }: { line: Line | null; tokens?: Tokens[number]; wrap: boolean; noted: boolean; picked: boolean; onPick?: () => void; gutter: "both" | "one" }) {
  if (!line) return <div className="min-h-5 bg-sunk" />;
  const pickable = !!onPick && (line.kind === "add" || line.kind === "del" || line.kind === "context");
  return <div className={`flex min-h-5 min-w-0 font-mono text-[12px] leading-5 ${tone(line.kind)} ${picked ? "outline-2 -outline-offset-2 outline-teal" : ""} ${pickable ? "cursor-pointer" : ""}`} onClick={pickable ? onPick : undefined}>
    <span className={`sticky left-0 flex shrink-0 justify-end gap-2 border-r border-line-2 bg-inherit px-2 text-ink-3 select-none ${gutter === "both" ? "w-[82px]" : "w-[48px]"} ${noted ? "bg-attn-soft! font-semibold text-attn" : ""}`}>
      {noted && <Icon name="pencil" size={12} className="mt-[5px]" />}
      {gutter === "both" && <span>{line.old}</span>}
      <span>{gutter === "both" ? line.next : line.next ?? line.old}</span>
    </span>
    <span className={`px-2 ${wrap ? "min-w-0 flex-1 break-all whitespace-pre-wrap" : "whitespace-pre"}`}>
      {tokens && line.kind !== "meta" && line.kind !== "hunk" ? tokens.map(([text, color, style], i) => <span key={i} style={{ color, fontStyle: style && style & 1 ? "italic" : undefined, fontWeight: style && style & 2 ? 600 : undefined }}>{text}</span>) : line.text || " "}
    </span>
  </div>;
});

/** The patch of one file: unified or split, optional wrap, clickable lines for notes. Scrolls inside its parent. */
export const PatchView = memo(function PatchView({ patch, path, language, split, wrap, notes, picked, onPick, maxHeight, below, jump }: { patch: string; path: string; language?: string; split: boolean; wrap: boolean; notes?: LineNote[]; picked?: number | null; onPick?: (line: Line) => void; maxHeight?: number; below?: (line: Line) => ReactNode; jump?: number | null }) {
  const theme = useTheme((s) => s.theme);
  const parsed = useMemo(() => parsePatch(patch), [patch]);
  const [tokens, setTokens] = useState<Tokens | null>(null);
  useEffect(() => {
    let active = true;
    setTokens(null);
    const text = parsed.map((l) => l.text).join("\n");
    if (text.length < 1_000_000 && parsed.length < 20_000) void highlight(`diff:${path}`, text, language ?? langOf(path), theme).then((t) => { if (active) setTokens(t); });
    return () => { active = false; };
  }, [path, language, parsed, theme]);
  const rows = useMemo(() => {
    if (!split) return parsed.map((l) => ({ left: l as Line | null, right: null as Line | null, both: true }));
    const out: { left: Line | null; right: Line | null; both: boolean }[] = [];
    for (let i = 0; i < parsed.length;) {
      const l = parsed[i];
      if (l.kind === "del" || l.kind === "add") {
        const removed: Line[] = [], added: Line[] = [];
        while (i < parsed.length && parsed[i].kind === "del") removed.push(parsed[i++]);
        while (i < parsed.length && parsed[i].kind === "add") added.push(parsed[i++]);
        for (let j = 0; j < Math.max(removed.length, added.length); j++) out.push({ left: removed[j] ?? null, right: added[j] ?? null, both: false });
      } else { out.push({ left: l, right: l, both: l.kind === "hunk" || l.kind === "meta" }); i++; }
    }
    return out;
  }, [parsed, split]);
  const isNoted = (l: Line | null) => {
    if (!l || !notes?.length) return false;
    const { line, side } = noteLine(l);
    return notes.some((n) => n.line === line && (n.side ? n.side === side : n.code === l.text));
  };
  const ref = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({ count: rows.length, getScrollElement: () => ref.current, estimateSize: () => 20, overscan: 8, useFlushSync: false, measureElement: (el) => el.getBoundingClientRect().height });
  useEffect(() => { ref.current?.scrollTo({ top: 0 }); }, [patch]);
  useEffect(() => { if (jump != null) { const at = rows.findIndex((r) => r.left?.next === jump || r.right?.next === jump || r.left?.old === jump); if (at >= 0) v.scrollToIndex(at, { align: "center" }); } }, [jump, rows]);
  const width = useMemo(() => wrap ? undefined : Math.max(...rows.map((r) => split ? 2 * Math.max(r.left?.text.length ?? 0, r.right?.text.length ?? 0) * 7.3 + 140 : (r.left?.text.length ?? 0) * 7.3 + 110), 240), [rows, wrap, split]);
  const cell = (l: Line | null, g: "both" | "one") => <Cell line={l} tokens={l ? tokens?.[l.index] : undefined} wrap={wrap} noted={isNoted(l)} picked={!!l && picked === l.index} onPick={onPick && l ? () => onPick(l) : undefined} gutter={g} />;
  return <div ref={ref} className={`min-h-0 overflow-auto ${maxHeight ? "" : "flex-1"}`} style={maxHeight ? { height: Math.min(maxHeight, rows.length * 20 + 20) } : undefined}>
    <div className="relative min-w-full" style={{ height: v.getTotalSize(), width: width ?? "100%" }}>
      {v.getVirtualItems().map((r) => {
        const row = rows[r.index];
        return <div key={r.key} data-index={r.index} ref={v.measureElement} className="absolute left-0 w-full" style={{ transform: `translateY(${r.start}px)` }}>
          {!split || row.both ? cell(row.left, split ? "one" : "both") : <div className="grid grid-cols-2">{cell(row.left, "one")}<div className="border-l border-line">{cell(row.right, "one")}</div></div>}
          {row.left && below?.(row.left)}
          {row.right && row.right !== row.left && below?.(row.right)}
        </div>;
      })}
    </div>
  </div>;
});

/** Read-only patch with a bounded height (commit diffs). Kept for callers that show one file without the review UI. */
export const FilePatch = memo(function FilePatch({ file, split, emptyMessage = "Binary file or patch unavailable. Open the full file or GitHub to inspect it." }: { file: { filename: string; patch?: string; language?: string }; split: boolean; emptyMessage?: string }) {
  if (!file.patch) return <Empty icon="info">{emptyMessage}</Empty>;
  return <div className="border-t border-line"><PatchView patch={file.patch} path={file.filename} language={file.language} split={split} wrap={false} maxHeight={520} /></div>;
});

/** Split/unified preference shared by every Changes view on this device. */
export function useSplitPreference() {
  const phone = usePhone();
  const [split, setSplit] = useState(() => !phone && localStorage.getItem("diff-view") === "split");
  return [split && !phone, (on: boolean) => { localStorage.setItem("diff-view", on ? "split" : "unified"); setSplit(on); }] as const;
}

// The phone's "Tap a line" hint shows on the first file only, then never again on this device (browser storage; the
// demo never writes it, so it shows once per load).
const HINT_KEY = "launchpad.diff-tap-hint-seen";
let hintSeen = (() => { if (isDemo) return false; try { return localStorage.getItem(HINT_KEY) === "1"; } catch { return false; } })();
/** Whether the hint shows for `file` (null: no hint wanted): only on the first file that wanted it. */
function useTapHint(file: string | null) {
  const [at, setAt] = useState<string | null>(null);
  useEffect(() => {
    if (file === null || hintSeen) return;
    hintSeen = true;
    setAt(file);
    if (!isDemo && storageAllowed()) try { localStorage.setItem(HINT_KEY, "1"); } catch { /* This window only. */ }
  }, [file]);
  return file !== null && (at === file || !hintSeen);
}

/** The phone's file list or one file, when the host keeps it in the address: `file` says one file shows; `open`
 * (from the list) adds a history entry, `next` replaces it and `list` goes back to the list. */
export type PhoneFileView = { file: boolean; open: (path: string) => void; next: (path: string) => void; list: () => void };

const FileRow = memo(function FileRow({ f, active, viewed, notes, onClick, phone }: { f: DiffEntry; active: boolean; viewed: boolean; notes: number; onClick: () => void; phone: boolean }) {
  return <button className={`flex w-full items-center gap-2 px-2 text-left text-[12px] ${phone ? "h-11 border-b border-line-2 text-[13px]" : "h-7"} ${active && !phone ? "bg-select" : "hover:bg-sunk"}`} title={`${f.previous ? `${f.previous} → ` : ""}${f.path}`} onClick={onClick}>
    <span className={`w-3 flex-none text-center font-mono text-[11px] ${f.status.startsWith("A") || f.status === "added" ? "text-gh-open" : f.status.startsWith("D") || f.status === "removed" ? "text-gh-closed" : "text-teal-ink"}`}>{f.status.trim().slice(0, 1).toUpperCase()}</span>
    <span className={`min-w-0 flex-1 truncate font-mono ${viewed ? "text-ink-3" : ""}`}>{f.path}</span>
    {notes > 0 && <span className="flex flex-none items-center gap-0.5 text-attn" title={`${notes} note${notes === 1 ? "" : "s"}`}><Icon name="pencil" size={12} />{notes}</span>}
    {f.additions != null && !phone && <span className="flex-none tabular-nums text-[11px]"><span className="text-gh-open">+{f.additions}</span> <span className="text-gh-closed">−{f.deletions ?? 0}</span></span>}
    {viewed && <Icon name="check" size={12} className="flex-none text-gh-open" />}
  </button>;
});

export function DiffReview({ files, selected, onSelect, patch, loading, error, emptyPatch, truncated, language, viewed, onToggleViewed, notes = [], onAddNote, noteLabel = "Note for the agent", notePlaceholder = "What should the agent change here?", toolbar, status, fileActions, banner, phoneView, phoneBar, onEditNote, onDeleteNote, draftKey, focusLine }: {
  files: DiffEntry[];
  selected: string | undefined;
  onSelect: (path: string) => void;
  /** Patch text of the selected file, `undefined` while it loads. */
  patch: string | undefined;
  loading?: boolean;
  error?: unknown;
  emptyPatch?: ReactNode;
  truncated?: boolean;
  language?: string;
  viewed: (path: string) => boolean;
  /** Absent or disabled while the patch loads: the host cannot tell what was viewed. */
  onToggleViewed?: (path: string) => void;
  notes?: LineNote[];
  onEditNote?: (note: LineNote, text: string) => void;
  onDeleteNote?: (note: LineNote) => void;
  draftKey?: string;
  focusLine?: number | null;
  onAddNote?: (note: LineNote) => void;
  noteLabel?: string;
  notePlaceholder?: string;
  toolbar?: ReactNode;
  status?: ReactNode;
  fileActions?: (path: string) => ReactNode;
  banner?: ReactNode;
  /** Phone: list or file kept by the host (the address), else in this component. */
  phoneView?: PhoneFileView;
  /** Phone: the host's main action in the bottom bar of the file view (a PR's Review). */
  phoneBar?: ReactNode;
}) {
  const phone = usePhone();
  const [split, setSplit] = useSplitPreference();
  const [wrap, setWrap] = useState(phone);
  const [ownList, setShowList] = useState(true);
  const showList = phoneView ? !phoneView.file : ownList;
  // A host without `phoneView` (session Changes, a commit): the opened file is an overlay, so back returns to the list.
  useBackClose(phone && !phoneView && !ownList, () => setShowList(true));
  type Picked = { index: number; line: number | null; side: "old" | "new"; code: string };
  // Unfinished comments, one per file: on Home when the host names a key (a PR or Local PR), else in this view only,
  // so the Changes of different agents or commits never share a draft.
  type Drafts = Record<string, { picked: Picked | null; text: string }>;
  const shared = useReviewState<Drafts>(draftKey ?? "line-draft:none", {});
  const [own, setOwn] = useState<Drafts>({});
  const drafts = draftKey ? shared : { value: own, set: setOwn };
  const fileDraft = drafts.value[selected ?? ""];
  const picked = fileDraft?.picked ?? null, draft = fileDraft?.text ?? "";
  const update = (change: (d: { picked: Picked | null; text: string }) => { picked: Picked | null; text: string }) => drafts.set((v) => {
    const { [selected ?? ""]: before, ...rest } = v;
    const after = change(before ?? { picked: null, text: "" });
    return after.picked || after.text ? { ...rest, [selected ?? ""]: after } : rest;
  });
  const setPicked = (picked: Picked | null) => update((d) => ({ ...d, picked }));
  const setDraft = (text: string) => update((d) => ({ ...d, text }));
  const listRef = useRef<HTMLDivElement>(null);
  const list = useVirtualizer({ count: files.length, getScrollElement: () => listRef.current, estimateSize: () => (phone ? 44 : 28), overscan: 6, useFlushSync: false });

  const viewedCount = files.filter((f) => viewed(f.path)).length;
  const at = files.findIndex((f) => f.path === selected);
  const entry = files[at];
  const fileNotes = notes.filter((n) => n.path === selected);
  const open = (p: string) => { onSelect(p); if (phoneView) phoneView.open(p); else setShowList(false); };
  const next = (p: string) => { onSelect(p); phoneView?.next(p); };
  const toList = () => { if (phoneView) phoneView.list(); else setShowList(true); };
  // Phone, one file: the summary row goes, Viewed, Wrap and Next file move to a bottom bar, the hint shows once.
  const phoneFile = phone && !showList && files.length > 0;
  const hintable = !!patch && !!onAddNote && !picked;
  const firstHint = useTapHint(phoneFile && hintable && selected ? selected : null);
  const viewedBox = selected && <label className="flex items-center gap-1" title="Shared with your other devices"><input type="checkbox" checked={viewed(selected)} disabled={!onToggleViewed} onChange={() => onToggleViewed?.(selected)} />Viewed</label>;
  const wrapChoice = <Choice active={wrap} title="Wrap long lines" onClick={() => setWrap(!wrap)}>Wrap</Choice>;
  const add = () => {
    if (!picked || !selected || !draft.trim() || !onAddNote) return;
    onAddNote({ path: selected, line: picked.line, side: picked.side, code: picked.code, text: draft.trim() });
    setDraft(""); setPicked(null);
  };
  const fileList = <div ref={listRef} className={`overflow-auto ${phone ? "min-h-0 flex-1" : "w-[240px] shrink-0 border-r border-line"}`}>
    <div className="relative" style={{ height: list.getTotalSize() }}>
      {list.getVirtualItems().map((r) => { const f = files[r.index]; return <div key={f.path} className="absolute left-0 w-full" style={{ transform: `translateY(${r.start}px)` }}><FileRow f={f} phone={phone} active={f.path === selected} viewed={viewed(f.path)} notes={notes.filter((n) => n.path === f.path).length} onClick={() => open(f.path)} /></div>; })}
    </div>
  </div>;
  return <section className="flex min-h-0 flex-1 flex-col">
    {!phoneFile && <div className="phone-diff-toolbar flex flex-wrap items-center gap-2 border-b border-line px-3 py-2 text-[12px]">
      <b>{files.length} file{files.length === 1 ? "" : "s"} changed</b>
      {files.length > 0 && <span className="flex items-center gap-1.5 text-ink-2" title="Files marked viewed (shared with your other devices)"><span className="inline-block h-1.5 w-16 overflow-hidden rounded bg-meter"><i className="block h-full bg-gh-open" style={{ width: `${(viewedCount / files.length) * 100}%` }} /></span>{viewedCount}/{files.length} viewed</span>}
      {status && <span className="text-ink-3">{status}</span>}
      {toolbar}
      <span className="ml-auto flex items-center gap-1.5">
        {!phone && <Choices label="Diff layout"><Choice active={!split} onClick={() => setSplit(false)}>Unified</Choice><Choice active={split} onClick={() => setSplit(true)}>Split</Choice></Choices>}
        {wrapChoice}
      </span>
    </div>}
    {banner}
    {!files.length ? loading ? <Loading what="changes" /> : error ? <ErrorState className="m-2" error={error} /> : null : <div className="flex min-h-0 flex-1">
      {(!phone || showList) && fileList}
      {(!phone || !showList) && <div className="flex min-w-0 flex-1 flex-col">
        <div className="flex flex-wrap items-center gap-1.5 border-b border-line px-2 py-1.5 text-[12px]">
          {phone && <button className="btn btn-sm" onClick={toList}><Icon name="chev" size={12} className="rotate-90" />Files</button>}
          <span className="min-w-0 flex-1 truncate font-mono" title={selected}>{entry?.previous && `${entry.previous} → `}{selected}</span>
          {selected && fileActions?.(selected)}
          {!phone && viewedBox}
        </div>
        {!!error && <ErrorState className="m-2" error={error} />}
        {loading && <Loading what={selected ?? "the patch"} />}
        {truncated && <p className="px-2 py-1 text-[12px] text-ink-3">Large diff: showing its beginning and end.</p>}
        {patch !== undefined && !patch && !loading && <Empty icon="info">{emptyPatch ?? "No text changes for this file."}</Empty>}
        {hintable && (!phone || firstHint) && <p className="border-b border-line-2 px-2 py-1 text-[12px] text-ink-3">{phone ? "Tap" : "Click"} a line to write a {noteLabel.toLowerCase()}.</p>}
        {!!patch && selected && <PatchView patch={patch} path={selected} language={language} split={split} wrap={wrap} notes={fileNotes} picked={picked?.index ?? null} jump={focusLine} below={(l) => <><div className="max-w-text">{fileNotes.filter((n) => n.line === noteLine(l).line && (n.side ?? "new") === noteLine(l).side).map((n, i) => <InlineNote key={i} note={n} onEdit={onEditNote} onDelete={onDeleteNote} />)}</div>{picked?.index === l.index && (picked && onAddNote && <div className="grid gap-1.5 border-t border-line bg-chrome p-2 text-[12px]">
          <span className="truncate">{noteLabel} on <span className="font-mono">{selected}:{picked.line}</span> <span className="font-mono text-ink-3">{picked.code.slice(0, 80)}</span></span>
          <textarea autoFocus aria-label={noteLabel} className="field-input h-16 resize-y py-1.5" value={draft} onChange={(e) => setDraft(e.target.value)} placeholder={notePlaceholder} />
          <div className="flex gap-2"><button className="btn btn-sm btn-primary" disabled={!draft.trim()} onClick={add}>Comment</button><button className="btn btn-sm" onClick={() => { setPicked(null); setDraft(""); }}>Cancel</button></div>
        </div>)}</>} onPick={onAddNote ? (l) => setPicked({ index: l.index, ...noteLine(l), code: l.text }) : undefined} />}

        {phoneFile && !picked && <div className="flex flex-none flex-wrap items-center gap-2 border-t border-line bg-chrome px-3 py-1 text-[12px]">
          {viewedBox}
          {wrapChoice}
          <span className="ml-auto flex items-center gap-2">
            {at >= 0 && at < files.length - 1 && <button className="btn btn-sm" onClick={() => next(files[at + 1].path)}>Next file<Icon name="chev" size={12} className="-rotate-90" /></button>}
            {phoneBar}
          </span>
        </div>}
      </div>}
    </div>}
  </section>;
}

function InlineNote({ note, onEdit, onDelete }: { note: LineNote; onEdit?: (n: LineNote, text: string) => void; onDelete?: (n: LineNote) => void }) {
  const [editing, setEditing] = useState(false), [text, setText] = useState(note.text);
  return <div className="grid gap-2 border-y border-line bg-chrome p-3 text-[12px]">
    <b>You · line {note.line}</b>
    {editing ? <textarea aria-label="Edit line comment" className="field-input min-h-16" value={text} onChange={(e) => setText(e.target.value)} /> : <span className="whitespace-pre-wrap">{note.text}</span>}
    <div className="flex gap-2">{onEdit && <button className="btn btn-sm" disabled={editing && !text.trim()} onClick={() => { if (editing) onEdit(note, text); else setText(note.text); setEditing(!editing); }}>{editing ? "Save" : "Edit"}</button>}{onDelete && <button className="btn btn-sm" onClick={() => onDelete(note)}>Delete</button>}</div>
  </div>;
}
