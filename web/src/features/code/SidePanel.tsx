import { isPhone } from "@/lib/phone";
// Left panel of the code viewer: the file tree with "Go to file" and "Changed only", and the search
// in the project. Both lists are virtualized with fixed row heights.

import { useQuery } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useDeferredValue, useEffect, useMemo, useRef, useState } from "react";
import type { FileList } from "@/gen/FileList";
import type { SearchResult } from "@/gen/SearchResult";
import { Icon } from "@/components/Icon";
import { api } from "@/lib/api";
import { ancestors, basename, buildTree, dirname, filterFiles, flatten, lineRanges, matcher, sourceParams, type FindOpts, type TreeRow } from "./lib";
import { useCode } from "./store";

const TREE_ROW = isPhone ? 44 : 24;

export function Toggle({ on, onClick, title, children }: { on: boolean; onClick: () => void; title: string; children: React.ReactNode }) {
  return (
    <button
      title={title}
      onClick={onClick}
      className={`inline-grid h-6 min-w-6 place-items-center rounded-[5px] border px-1 font-mono text-[11px] font-semibold ${on ? "border-teal bg-teal-soft text-teal-ink" : "border-transparent text-ink-2 hover:bg-sunk"}`}
    >
      {children}
    </button>
  );
}

export function FindToggles({ opts, set }: { opts: FindOpts; set: (o: FindOpts) => void }) {
  return (
    <>
      <Toggle on={opts.case} title="Match case" onClick={() => set({ ...opts, case: !opts.case })}>
        Aa
      </Toggle>
      <Toggle on={opts.word} title="Whole word" onClick={() => set({ ...opts, word: !opts.word })}>
        <u>ab</u>
      </Toggle>
      <Toggle on={opts.regex} title="Regular expression" onClick={() => set({ ...opts, regex: !opts.regex })}>
        .*
      </Toggle>
    </>
  );
}

const TreeLine = memo(function TreeLine({ row, letter, changedDir, active, onClick }: { row: TreeRow; letter: string | undefined; changedDir: boolean; active: boolean; onClick: (r: TreeRow) => void }) {
  const pad = 10 + row.depth * 14 + (row.kind === "file" ? 18 : 0);
  return (
    <button
      onClick={() => onClick(row)}
      title={row.path}
      className={`code-tree-line flex h-6 w-full items-center gap-1.5 pr-2.5 text-left text-[13px] whitespace-nowrap ${active ? "bg-select font-semibold shadow-[inset_2px_0_0_var(--color-teal)]" : "hover:bg-sunk"} ${letter ? "text-attn" : "text-ink"}`}
      style={{ paddingLeft: pad }}
    >
      {row.kind === "dir" && <Icon name="chev" size={12} className={`text-ink-3 ${row.open ? "" : "-rotate-90"}`} />}
      <Icon name={row.kind === "dir" ? "folder" : "file"} size={13} className="text-ink-3" />
      <span className={`min-w-0 truncate ${row.kind === "dir" ? "font-[550]" : ""}`}>{row.name}</span>
      {changedDir && <span className="size-1.5 flex-none rounded-full bg-attn" title="Changed files inside" />}
      {letter && <span className="ml-auto font-mono text-[11px] font-bold">{letter}</span>}
    </button>
  );
});

const GoLine = memo(function GoLine({ path, letter, active, onClick }: { path: string; letter: string | undefined; active: boolean; onClick: (p: string) => void }) {
  return (
    <button
      onClick={() => onClick(path)}
      title={path}
      className={`code-tree-line flex h-6 w-full items-center gap-1.5 px-2.5 text-left text-[13px] whitespace-nowrap ${active ? "bg-select" : "hover:bg-sunk"} ${letter ? "text-attn" : ""}`}
    >
      <Icon name="file" size={13} className="text-ink-3" />
      <span className="flex-none font-[550]">{basename(path)}</span>
      <span className="min-w-0 truncate text-[12px] text-ink-3">{dirname(path)}</span>
      {letter && <span className="ml-auto font-mono text-[11px] font-bold">{letter}</span>}
    </button>
  );
});

export function FilesPanel({ list, skey, active, onOpen }: { list: FileList | undefined; skey: string; active: string | null; onOpen: (path: string) => void }) {
  const [query, setQuery] = useState("");
  const [changedOnly, setChangedOnly] = useState(false);
  const deferred = useDeferredValue(query);
  const expandedList = useCode((s) => s.sources[skey]?.expanded);
  const toggle = useCode((s) => s.toggle);
  const expand = useCode((s) => s.expand);
  const files = useMemo(() => list?.files ?? [], [list]);
  const changes = useMemo(() => list?.changes ?? {}, [list]);
  const changedCount = Object.keys(changes).length;
  const tree = useMemo(() => buildTree(files), [files]);
  const changedTree = useMemo(() => buildTree(files.filter((f) => changes[f])), [files, changes]);
  const changedDirs = useMemo(() => new Set(Object.keys(changes).flatMap(ancestors)), [changes]);
  const expanded = useMemo(() => new Set(expandedList ?? []), [expandedList]);
  const rows = useMemo(() => (changedOnly ? flatten(changedTree, expanded, true) : flatten(tree, expanded)), [changedOnly, changedTree, tree, expanded]);
  const found = useMemo(() => filterFiles(files, deferred), [files, deferred]);
  const going = query.trim() !== "";

  // Reveal the open file.
  useEffect(() => {
    if (active) expand(skey, ancestors(active));
  }, [active, skey, expand]);

  const ref = useRef<HTMLDivElement>(null);
  const count = going ? found.length : rows.length;
  const v = useVirtualizer({ count, getScrollElement: () => ref.current, estimateSize: () => TREE_ROW, overscan: 8, useFlushSync: false });
  const revealed = useRef<string | null>(null);
  useEffect(() => {
    if (!active || going || revealed.current === `${skey}\n${active}`) return;
    const i = rows.findIndex((r) => r.path === active);
    if (i >= 0) {
      revealed.current = `${skey}\n${active}`;
      v.scrollToIndex(i, { align: "auto" });
    }
  }, [active, rows, going, skey, v]);

  const onRow = useMemo(() => (r: TreeRow) => (r.kind === "dir" ? toggle(skey, r.path) : onOpen(r.path)), [toggle, skey, onOpen]);

  return (
    <>
      <div className="grid grid-cols-[minmax(0,1fr)] gap-2 border-b border-line-2 px-2.5 pt-1 pb-2">
        <div className="flex h-[30px] items-center gap-[7px] rounded-[7px] border border-line px-2 text-ink-3 focus-within:border-teal">
          <Icon name="search" size={13} />
          <input className="min-w-0 flex-1 bg-transparent text-[13px] text-ink outline-none" placeholder="Go to file" value={query} onChange={(e) => setQuery(e.target.value)} />
          {going && (
            <button className="text-ink-3 hover:text-ink" title="Clear" onClick={() => setQuery("")}>
              <Icon name="x" size={12} />
            </button>
          )}
        </div>
        <label className={`flex items-center gap-2 text-xs whitespace-nowrap text-ink-2 ${list?.git === false ? "opacity-50" : ""}`}>
          <input type="checkbox" className="accent-teal" checked={changedOnly} disabled={!list?.git} onChange={(e) => setChangedOnly(e.target.checked)} />
          Changed only
          <span className="ml-auto text-[12px] text-ink-3" title={list?.base ? `Since the branch left ${list.base}, committed or not` : undefined}>
            {list?.git === false ? "not a git repository" : `${changedCount} changed${list?.base ? ` since ${list.base.replace(/^origin\//, "")}` : ""}`}
          </span>
        </label>
      </div>
      <div ref={ref} className="min-h-0 flex-1 overflow-y-auto py-1">
        {!list && <div className="px-3 py-2 text-xs text-ink-3">Loading files…</div>}
        {going && found.length === 0 && <div className="px-3 py-2 text-xs text-ink-3">No file matches.</div>}
        {changedOnly && !going && rows.length === 0 && list && <div className="px-3 py-2 text-xs text-ink-3">No changed file.</div>}
        <div className="relative" style={{ height: v.getTotalSize() }}>
          {v.getVirtualItems().map((it) => (
            <div key={it.key} className="absolute top-0 left-0 w-full" style={{ height: TREE_ROW, transform: `translateY(${it.start}px)` }}>
              {going ? (
                <GoLine path={found[it.index]} letter={changes[found[it.index]]} active={found[it.index] === active} onClick={onOpen} />
              ) : (
                <TreeLine
                  row={rows[it.index]}
                  letter={rows[it.index].kind === "file" ? changes[rows[it.index].path] : undefined}
                  changedDir={rows[it.index].kind === "dir" && !rows[it.index].open && changedDirs.has(rows[it.index].path)}
                  active={rows[it.index].path === active}
                  onClick={onRow}
                />
              )}
            </div>
          ))}
        </div>
      </div>
    </>
  );
}

// ---------------------------------------------------------------- search in project

type SearchRow = { t: "file"; path: string; count: number } | { t: "hit"; path: string; line: number; text: string } | { t: "more"; path: string; hidden: number };

export function SearchPanel({ root, rev, onOpen, state, setState }: { root: string; rev: string | null; onOpen: (path: string, line: number) => void; state: { q: string; opts: FindOpts }; setState: (s: { q: string; opts: FindOpts }) => void }) {
  const [shown, setShown] = useState<Set<string>>(new Set());
  const q = useDeferredDebounce(state.q, 250);
  const { opts } = state;
  const enabled = q.length >= 2;
  const res = useQuery({
    queryKey: ["code-search", root, rev, q, opts],
    queryFn: ({ signal }) =>
      api.get<SearchResult>(`/api/code/search?${sourceParams(root, rev)}&q=${encodeURIComponent(q)}&case=${opts.case}&word=${opts.word}&regex=${opts.regex}`, signal),
    enabled,
    staleTime: 10_000,
    retry: false,
  });
  const re = useMemo(() => {
    const m = matcher(q, opts);
    return m instanceof RegExp ? m : null;
  }, [q, opts]);
  const rows = useMemo(() => {
    const groups = new Map<string, { line: number; text: string }[]>();
    for (const h of res.data?.hits ?? []) {
      const g = groups.get(h.path);
      if (g) g.push(h);
      else groups.set(h.path, [h]);
    }
    const out: SearchRow[] = [];
    for (const [path, hits] of groups) {
      out.push({ t: "file", path, count: hits.length });
      const all = shown.has(path);
      for (const h of all ? hits : hits.slice(0, 4)) out.push({ t: "hit", path, line: h.line, text: h.text });
      if (!all && hits.length > 4) out.push({ t: "more", path, hidden: hits.length - 4 });
    }
    return { out, files: groups.size };
  }, [res.data, shown]);
  useEffect(() => setShown(new Set()), [q, opts, root, rev]);

  const ref = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({ count: rows.out.length, getScrollElement: () => ref.current, estimateSize: () => TREE_ROW, overscan: 8, useFlushSync: false });
  const total = res.data?.hits.length ?? 0;
  return (
    <>
      <div className="grid grid-cols-[minmax(0,1fr)] gap-2 border-b border-line-2 px-2.5 pt-1 pb-2">
        <div className="flex h-[30px] items-center gap-1 rounded-[7px] border border-line pr-1 pl-2 text-ink-3 focus-within:border-teal">
          <Icon name="search" size={13} />
          <input className="ml-1 min-w-0 flex-1 bg-transparent text-[13px] text-ink outline-none" placeholder="Search in project" value={state.q} onChange={(e) => setState({ ...state, q: e.target.value })} />
          <FindToggles opts={opts} set={(o) => setState({ ...state, opts: o })} />
        </div>
        <div className="text-[12px] text-ink-3">
          {!enabled
            ? "Type at least two characters. Uses git grep on this source."
            : res.isFetching
              ? "Searching…"
              : res.isError
                ? <span className="text-gh-closed">{(res.error as Error).message}</span>
                : total === 0
                  ? "No results"
                  : `${res.data?.truncated ? "First 1,000" : total.toLocaleString()} results in ${rows.files} file${rows.files === 1 ? "" : "s"}`}
        </div>
      </div>
      <div ref={ref} className="min-h-0 flex-1 overflow-y-auto py-1">
        <div className="relative" style={{ height: v.getTotalSize() }}>
          {v.getVirtualItems().map((it) => {
            const r = rows.out[it.index];
            return (
              <div key={it.key} className="absolute top-0 left-0 w-full" style={{ height: TREE_ROW, transform: `translateY(${it.start}px)` }}>
                {r.t === "file" ? (
                  <button className="flex h-6 w-full items-center gap-1.5 px-2.5 text-left text-xs font-[650] hover:bg-sunk" title={r.path} onClick={() => onOpen(r.path, 1)}>
                    <Icon name="file" size={13} className="text-ink-3" />
                    <span className="min-w-0 truncate">{r.path}</span>
                    <span className="ml-auto font-normal text-ink-3 tabular-nums">{r.count}</span>
                  </button>
                ) : r.t === "hit" ? (
                  <HitLine path={r.path} line={r.line} text={r.text} re={re} onOpen={onOpen} />
                ) : (
                  <button className="h-6 w-full pl-8 text-left text-xs font-semibold text-teal-ink hover:underline" onClick={() => setShown((s) => new Set(s).add(r.path))}>
                    Show {r.hidden} more
                  </button>
                )}
              </div>
            );
          })}
        </div>
      </div>
    </>
  );
}

const HitLine = memo(function HitLine({ path, line, text, re, onOpen }: { path: string; line: number; text: string; re: RegExp | null; onOpen: (path: string, line: number) => void }) {
  const t = text.trimStart();
  const cut = text.length - t.length;
  const ranges = lineRanges(text, re);
  const parts: React.ReactNode[] = [];
  let at = cut;
  for (const [a, b] of ranges) {
    if (b <= at) continue;
    if (a > at) parts.push(text.slice(at, a));
    parts.push(<mark key={a} className="cv-m">{text.slice(Math.max(a, at), b)}</mark>);
    at = b;
  }
  parts.push(text.slice(at));
  return (
    <button className="flex h-6 w-full items-center gap-2 pr-2.5 pl-8 text-left hover:bg-sunk" title={`${path}:${line}`} onClick={() => onOpen(path, line)}>
      <span className="w-8 flex-none text-right font-mono text-[11px] text-ink-3 tabular-nums">{line}</span>
      <span className="min-w-0 truncate font-mono text-[12px] whitespace-pre">{parts}</span>
    </button>
  );
});

function useDeferredDebounce<T>(value: T, ms: number): T {
  const [v, setV] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setV(value), ms);
    return () => clearTimeout(t);
  }, [value, ms]);
  return v;
}
