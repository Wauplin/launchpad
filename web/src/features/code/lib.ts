// Code viewer helpers: API queries, the file tree, "Go to file", find in file and `path:line` links.

import { useQuery } from "@tanstack/react-query";
import type { CodeFile } from "@/gen/CodeFile";
import type { CodeSources } from "@/gen/CodeSources";
import type { CommitInfo } from "@/gen/CommitInfo";
import type { FileList } from "@/gen/FileList";
import { api } from "@/lib/api";

export const sourceParams = (root: string, rev: string | null | undefined) => `root=${encodeURIComponent(root)}${rev ? `&rev=${encodeURIComponent(rev)}` : ""}`;

export const rawUrl = (root: string, rev: string | null, path: string) => `/api/code/raw?${sourceParams(root, rev)}&path=${encodeURIComponent(path)}`;

export const useCodeSources = (projectId: string | undefined) =>
  useQuery({ queryKey: ["code-sources", projectId], queryFn: () => api.get<CodeSources>(`/api/code/sources?project=${projectId}`), enabled: !!projectId, staleTime: 30_000 });

/** The whole file list of a source. The viewer reloads it when it opens. */
export const useCodeFiles = (root: string | null | undefined, rev: string | null, opts: { always?: boolean } = {}) =>
  useQuery({
    queryKey: ["code-files", root, rev],
    queryFn: () => api.get<FileList>(`/api/code/files?${sourceParams(root!, rev)}`),
    enabled: !!root,
    staleTime: 60_000,
    refetchOnMount: opts.always ? "always" : true,
    retry: false,
  });

export const useCodeFile = (root: string, rev: string | null, path: string | null | undefined) =>
  useQuery({
    queryKey: ["code-file", root, rev, path],
    queryFn: () => api.get<CodeFile>(`/api/code/file?${sourceParams(root, rev)}&path=${encodeURIComponent(path!)}`),
    enabled: !!path,
    staleTime: 0,
    retry: false,
  });

export const useCommit = (root: string, rev: string | null) =>
  useQuery({ queryKey: ["code-commit", root, rev], queryFn: () => api.get<CommitInfo>(`/api/code/commit?${sourceParams(root, rev)}`), staleTime: 60_000 });

export const basename = (p: string) => p.slice(p.lastIndexOf("/") + 1);
export const dirname = (p: string) => (p.includes("/") ? p.slice(0, p.lastIndexOf("/")) : "");

/** `src/x.py:405-411` (or `:405`). */
export const lineRef = (path: string, sel: { start: number; end: number } | null) => (sel ? `${path}:${sel.start}${sel.end > sel.start ? `-${sel.end}` : ""}` : path);

export function githubUrl(commit: CommitInfo | undefined, path: string, sel?: { start: number; end: number } | null) {
  if (!commit?.github || !commit.sha) return null;
  const hash = sel ? `#L${sel.start}${sel.end > sel.start ? `-L${sel.end}` : ""}` : "";
  return `https://github.com/${commit.github}/blob/${commit.sha}/${path.split("/").map(encodeURIComponent).join("/")}${hash}`;
}

// ---------------------------------------------------------------- tree

export type Dir = { name: string; path: string; dirs: Dir[]; files: string[] };
export type TreeRow = { kind: "dir" | "file"; path: string; name: string; depth: number; open: boolean };

type DirBuild = { name: string; path: string; dirs: Map<string, DirBuild>; files: string[] };

const byName = (a: string, b: string) => {
  const x = a.toLowerCase();
  const y = b.toLowerCase();
  return x < y ? -1 : x > y ? 1 : 0;
};

export function buildTree(files: string[]): Dir {
  const root: DirBuild = { name: "", path: "", dirs: new Map(), files: [] };
  for (const f of files) {
    let d = root;
    let start = 0;
    for (let i = f.indexOf("/"); i >= 0; i = f.indexOf("/", start)) {
      const name = f.slice(start, i);
      let sub = d.dirs.get(name);
      if (!sub) {
        sub = { name, path: f.slice(0, i), dirs: new Map(), files: [] };
        d.dirs.set(name, sub);
      }
      d = sub;
      start = i + 1;
    }
    d.files.push(f);
  }
  const freeze = (d: DirBuild): Dir => ({
    name: d.name,
    path: d.path,
    dirs: [...d.dirs.values()].sort((a, b) => byName(a.name, b.name)).map(freeze),
    files: d.files.sort((a, b) => byName(basename(a), basename(b))),
  });
  return freeze(root);
}

/** Visible rows. A folder with a single child folder shares its row (`src/huggingface_hub`). */
export function flatten(root: Dir, expanded: Set<string>, all = false): TreeRow[] {
  const rows: TreeRow[] = [];
  const walk = (d: Dir, depth: number) => {
    for (let sub of d.dirs) {
      let name = sub.name;
      while (sub.dirs.length === 1 && sub.files.length === 0) {
        sub = sub.dirs[0];
        name += `/${sub.name}`;
      }
      const open = all || expanded.has(sub.path);
      rows.push({ kind: "dir", path: sub.path, name, depth, open });
      if (open) walk(sub, depth + 1);
    }
    for (const f of d.files) rows.push({ kind: "file", path: f, name: basename(f), depth, open: false });
  };
  walk(root, 0);
  return rows;
}

/** Every folder above a path, to reveal it in the tree. */
export function ancestors(path: string): string[] {
  const out: string[] = [];
  for (let i = path.indexOf("/"); i >= 0; i = path.indexOf("/", i + 1)) out.push(path.slice(0, i));
  return out;
}

/** "Go to file": every word must appear in the path; file name matches and short paths first. */
export function filterFiles(files: string[], query: string, limit = 400): string[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return [];
  const last = words[words.length - 1];
  const hits: [number, string][] = [];
  for (const f of files) {
    const lower = f.toLowerCase();
    if (!words.every((w) => lower.includes(w))) continue;
    const name = lower.slice(lower.lastIndexOf("/") + 1);
    hits.push([(name.startsWith(last) ? 0 : name.includes(last) ? 1 : 2) * 10_000 + f.length, f]);
  }
  hits.sort((a, b) => a[0] - b[0]);
  return hits.slice(0, limit).map((h) => h[1]);
}

// ---------------------------------------------------------------- find

export type FindOpts = { case: boolean; word: boolean; regex: boolean };
export type Match = { line: number; start: number; end: number };

/** The regular expression for a query, or an error message. Whole word follows `git grep -w`. */
export function matcher(q: string, o: FindOpts): RegExp | string | null {
  if (!q) return null;
  let src = o.regex ? q : q.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  if (o.word) src = o.regex ? `\\b(?:${src})\\b` : `${/^\w/.test(q) ? "\\b" : ""}${src}${/\w$/.test(q) ? "\\b" : ""}`;
  try {
    return new RegExp(src, o.case ? "g" : "gi");
  } catch {
    return "Invalid regular expression";
  }
}

/** Matches in the whole file (not only the lines on screen), at most `cap`. */
export function findAll(lines: string[], re: RegExp, cap = 10_000): Match[] {
  const out: Match[] = [];
  for (let i = 0; i < lines.length && out.length < cap; i++) {
    const text = lines[i];
    re.lastIndex = 0;
    for (let m = re.exec(text); m; m = re.exec(text)) {
      if (m[0].length === 0) {
        re.lastIndex++;
        continue;
      }
      out.push({ line: i, start: m.index, end: m.index + m[0].length });
      if (out.length >= cap) break;
    }
  }
  return out;
}

/** Match ranges of one line, for highlighting search results. */
export function lineRanges(text: string, re: RegExp | null): [number, number][] {
  if (!re) return [];
  return findAll([text], re, 50).map((m) => [m.start, m.end]);
}

// ---------------------------------------------------------------- links from sessions

export type PathHit = { path: string; line: number | null };

/** Resolves `hf_file_system.py:407`, `src/x.py` or an absolute path to a file of the list. */
export function pathResolver(files: string[], cwd: string): (raw: string) => PathHit | null {
  const set = new Set(files);
  const byBase = new Map<string, string[]>();
  for (const f of files) {
    const b = basename(f);
    const list = byBase.get(b);
    if (list) list.push(f);
    else byBase.set(b, [f]);
  }
  return (raw) => {
    const m = /^([^\s:'"`<>]+?)(?::(\d+)(?:-\d+)?)?(?::\d+)?[.,;)]?$/.exec(raw.trim());
    if (!m) return null;
    let p = m[1];
    if (p.startsWith(`${cwd}/`)) p = p.slice(cwd.length + 1);
    p = p.replace(/^\.\//, "");
    const line = m[2] ? Number(m[2]) : null;
    if (set.has(p)) return { path: p, line };
    if (p.startsWith("/") || !/\.\w+$/.test(p)) return null;
    const same = (byBase.get(basename(p)) ?? []).filter((f) => f === p || f.endsWith(`/${p}`));
    return same.length === 1 ? { path: same[0], line } : null;
  };
}
