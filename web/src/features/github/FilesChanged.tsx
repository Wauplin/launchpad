import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useEffect, useMemo, useRef, useState } from "react";
import { api } from "@/lib/api";
import { useTheme } from "@/stores/theme";
import { highlight, type Tokens } from "@/features/code/highlight";
import type { Diff, DiffFile } from "./types";
import { itemPath, openGitHub } from "./lib";
import { useReviewState, viewedKey } from "@/lib/reviewState";
type Line = {
  text: string;
  kind: string;
  old: number | null;
  next: number | null;
  index: number;
};
function lines(patch: string): Line[] {
  let old = 0,
    next = 0;
  return patch
    .split("\n")
    .filter((s, i, a) => i < a.length - 1 || s !== "")
    .map((text, index) => {
      const m = /^@@ -(\d+)(?:,\d+)? \+(\d+)/.exec(text);
      if (m) {
        old = Number(m[1]);
        next = Number(m[2]);
        return { text, kind: "hunk", old: null, next: null, index };
      }
      const kind = text.startsWith("+")
        ? "add"
        : text.startsWith("-")
          ? "del"
          : text.startsWith("\\")
            ? "note"
            : "context";
      return {
        text: kind === "note" ? text : text.slice(1),
        kind,
        old: kind === "add" || kind === "note" ? null : old++,
        next: kind === "del" || kind === "note" ? null : next++,
        index,
      };
    });
}
function lang(path: string) {
  const ext = path.split(".").pop() ?? "";
  return (
    (
      {
        rs: "rust",
        py: "python",
        ts: "typescript",
        tsx: "tsx",
        js: "javascript",
        jsx: "jsx",
        json: "json",
        md: "markdown",
        toml: "toml",
        yml: "yaml",
        yaml: "yaml",
        css: "css",
        html: "html",
        sh: "bash",
        c: "c",
        h: "c",
        cpp: "cpp",
        go: "go",
      } as Record<string, string>
    )[ext] ?? "text"
  );
}
const Cell = memo(function Cell({
  line,
  tokens,
}: {
  line: Line;
  tokens?: Tokens[number];
}) {
  return (
    <div
      className={`flex h-5 min-w-0 font-mono text-[12px] ${line.kind === "add" ? "bg-diff-add" : line.kind === "del" ? "bg-diff-del" : line.kind === "hunk" ? "bg-work-soft" : ""}`}
    >
      <span className="sticky left-0 flex w-[82px] shrink-0 justify-end gap-2 border-r border-line-2 bg-inherit px-2 text-ink-3">
        <span>{line.old}</span>
        <span>{line.next}</span>
      </span>
      <span className="px-2 whitespace-pre">
        {tokens
          ? tokens.map(([text, color, style], i) => (
              <span
                key={i}
                style={{
                  color,
                  fontStyle: style && style & 1 ? "italic" : undefined,
                  fontWeight: style && style & 2 ? 600 : undefined,
                }}
              >
                {text}
              </span>
            ))
          : line.text}
      </span>
    </div>
  );
});
export const FilePatch = memo(function FilePatch({
  file,
  split,
  emptyMessage = "Binary file or patch unavailable. Open the full file or GitHub to inspect it.",
}: {
  file: DiffFile;
  split: boolean;
  emptyMessage?: string;
}) {
  const theme = useTheme((s) => s.theme);
  const parsed = useMemo(() => lines(file.patch ?? ""), [file.patch]);
  const [tokens, setTokens] = useState<Tokens | null>(null);
  const ref = useRef<HTMLDivElement>(null);
  const rightRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    let active = true;
    setTokens(null);
    const text = parsed.map((l) => l.text).join("\n");
    if (text.length < 1_000_000 && parsed.length < 20_000)
      void highlight(
        `diff:${file.filename}`,
        text,
        file.language ?? lang(file.filename),
        theme,
      ).then((t) => {
        if (active) setTokens(t);
      });
    return () => {
      active = false;
    };
  }, [file.filename, file.language, parsed, theme]);
  const rows = useMemo(() => {
    if (!split) return parsed.map((l) => ({ left: l, right: null }));
    const out: { left: Line | null; right: Line | null }[] = [];
    for (let i = 0; i < parsed.length;) {
      const l = parsed[i];
      if (l.kind === "del" || l.kind === "add") {
        const removed: Line[] = [],
          added: Line[] = [];
        while (i < parsed.length && parsed[i].kind === "del")
          removed.push(parsed[i++]);
        while (i < parsed.length && parsed[i].kind === "add")
          added.push(parsed[i++]);
        for (let j = 0; j < Math.max(removed.length, added.length); j++)
          out.push({ left: removed[j] ?? null, right: added[j] ?? null });
      } else {
        out.push({ left: l, right: l });
        i++;
      }
    }
    return out;
  }, [parsed, split]);
  const v = useVirtualizer({
    count: rows.length,
    getScrollElement: () => ref.current,
    estimateSize: () => 20,
    overscan: 8,
    useFlushSync: false,
  });
  const rightV = useVirtualizer({
    count: split ? rows.length : 0,
    getScrollElement: () => rightRef.current,
    estimateSize: () => 20,
    overscan: 8,
    useFlushSync: false,
  });
  const widths = useMemo(
    () =>
      rows.reduce(
        (out, row) => ({
          left: Math.max(out.left, (row.left?.text.length ?? 0) * 7.3 + 100),
          right: Math.max(out.right, (row.right?.text.length ?? 0) * 7.3 + 100),
        }),
        { left: 240, right: 240 },
      ),
    [rows],
  );
  return (
    <>
      <div
        className={
          split
            ? "grid grid-cols-2 border-t border-line"
            : "border-t border-line"
        }
        style={{ height: Math.min(520, rows.length * 20 + 20) }}
      >
        <div
          ref={ref}
          className="min-w-0 overflow-auto border-r border-line"
          onScroll={(e) => {
            if (
              rightRef.current &&
              rightRef.current.scrollTop !== e.currentTarget.scrollTop
            )
              rightRef.current.scrollTop = e.currentTarget.scrollTop;
          }}
        >
          <div
            className="relative min-w-full"
            style={{ height: v.getTotalSize(), width: widths.left }}
          >
            {v.getVirtualItems().map((r) => {
              const line = rows[r.index].left;
              return (
                <div
                  key={r.key}
                  className="absolute w-full"
                  style={{ top: r.start }}
                >
                  {line ? (
                    <Cell line={line} tokens={tokens?.[line.index]} />
                  ) : (
                    <div className="h-5 bg-sunk" />
                  )}
                </div>
              );
            })}
          </div>
        </div>
        {split && (
          <div
            ref={rightRef}
            className="min-w-0 overflow-auto"
            onScroll={(e) => {
              if (
                ref.current &&
                ref.current.scrollTop !== e.currentTarget.scrollTop
              )
                ref.current.scrollTop = e.currentTarget.scrollTop;
            }}
          >
            <div
              className="relative min-w-full"
              style={{ height: rightV.getTotalSize(), width: widths.right }}
            >
              {rightV.getVirtualItems().map((r) => {
                const line = rows[r.index].right;
                return (
                  <div
                    key={r.key}
                    className="absolute w-full"
                    style={{ top: r.start }}
                  >
                    {line ? (
                      <Cell line={line} tokens={tokens?.[line.index]} />
                    ) : (
                      <div className="h-5 bg-sunk" />
                    )}
                  </div>
                );
              })}
            </div>
          </div>
        )}
      </div>
      {!file.patch && (
        <p className="p-3 text-ink-3">
          {emptyMessage}
        </p>
      )}
    </>
  );
});
export function FilesChanged({
  repo,
  number,
  head,
  local,
}: {
  repo: string;
  number: number;
  head: string;
  local?: { id: string; root: string };
}) {
  const q = useQuery({
    queryKey: local ? ["predrafts", local.id, "diff", head] : ["pr-diff", repo, number, head],
    queryFn: () => local ? api.get<Diff>(`/api/predrafts/${local.id}`) : api.post<Diff>(`${itemPath(repo, number)}/diff`),
    staleTime: Infinity,
  });
  const [split, setSplit] = useState(
    () => localStorage.getItem("diff-view") === "split",
  );
  const [selected, setSelected] = useState(0);
  // Viewed markers live in the daemon so every device shares review progress.
  const viewed = useReviewState<string[]>(viewedKey(local?.id ?? repo, number, head), []);
  const files = q.data?.files ?? [];
  const listed = files[selected];
  // PR file lists come without patches: load the selected file's patch on demand.
  const lazy = !!listed && listed.patch === undefined && !!listed.has_patch && !local;
  const patch = useQuery({
    queryKey: ["pr-diff-file", repo, number, head, listed?.filename],
    queryFn: () => api.get<DiffFile>(`${itemPath(repo, number)}/diff/file?${new URLSearchParams({ base: q.data!.base, head: q.data!.head, project: q.data!.project_id ?? "api", path: listed!.filename })}`),
    enabled: lazy,
    staleTime: Infinity,
  });
  const file = lazy ? patch.data : listed;
  const ref = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({
    count: files.length,
    getScrollElement: () => ref.current,
    estimateSize: () => 40,
    overscan: 3,
    useFlushSync: false,
  });
  return (
    <section className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center gap-2 border-b border-line p-3">
        <b>{files.length} files changed</b>
        <span className="text-xs text-ink-3">
          {q.data?.fallback
            ? "GitHub patches (large patches may be truncated)"
            : "Local git diff"}
        </span>
        <button
          className="btn btn-sm ml-auto"
          onClick={() => {
            localStorage.setItem("diff-view", split ? "unified" : "split");
            setSplit(!split);
          }}
        >
          {split ? "Unified view" : "Split view"}
        </button>
      </div>
      {q.error && <p className="p-4 text-gh-closed">{String(q.error)}</p>}
      {q.isPending && <p className="p-4 text-ink-3">Fetching PR changes…</p>}
      <div className="flex min-h-0 flex-1">
        <div
          ref={ref}
          className="w-[240px] flex-none overflow-auto border-r border-line"
        >
          <div className="relative" style={{ height: v.getTotalSize() }}>
            {v.getVirtualItems().map((r) => {
              const f = files[r.index];
              return (
                <div
                  key={f.filename}
                  className={`absolute flex h-10 w-full items-center gap-2 px-2 text-xs ${selected === r.index ? "bg-select" : ""}`}
                  style={{ top: r.start }}
                >
                  <input
                    type="checkbox"
                    aria-label={`Viewed ${f.filename}`}
                    checked={viewed.value.includes(f.filename)}
                    onChange={() =>
                      viewed.set((s) =>
                        s.includes(f.filename)
                          ? s.filter((n) => n !== f.filename)
                          : [...s, f.filename],
                      )
                    }
                  />
                  <button
                    className="min-w-0 flex-1 truncate text-left font-mono"
                    title={f.filename}
                    onClick={() => setSelected(r.index)}
                  >
                    {f.filename}
                  </button>
                  {f.additions != null && <span className="shrink-0 tabular-nums"><span className="text-gh-open">+{f.additions}</span> <span className="text-gh-closed">−{f.deletions}</span></span>}
                  <span>{f.status.slice(0, 1).toUpperCase()}</span>
                </div>
              );
            })}
          </div>
        </div>
        <div className="min-w-0 flex-1 overflow-auto">
          {lazy && patch.isPending && <p className="p-4 text-ink-3">Loading {listed.filename}…</p>}
          {lazy && patch.error && <p className="p-4 text-gh-closed">{String(patch.error)}</p>}
          {file && (
            <>
              <div className="flex items-center gap-2 p-3">
                <b
                  className="min-w-0 truncate font-mono text-xs"
                  title={file.filename}
                >
                  {file.previous_filename && `${file.previous_filename} → `}
                  {file.filename}
                </b>
                <span className="text-xs">{file.status}</span>
                {q.data?.project_id && (
                  <Link
                    className="btn btn-sm ml-auto no-underline"
                    to="/projects/$id/code"
                    params={{ id: q.data.project_id }}
                    search={{
                      root: local?.root,
                      rev:
                        file.status === "removed" ? q.data.base : q.data.head,
                      path: file.filename,
                    }}
                  >
                    View file
                  </Link>
                )}
                {!q.data?.project_id && (
                  <button
                    className="btn btn-sm ml-auto"
                    onClick={() =>
                      openGitHub(
                        `https://github.com/${repo}/blob/${file.status === "removed" ? q.data?.base : q.data?.head}/${file.filename.split("/").map(encodeURIComponent).join("/")}`,
                      )
                    }
                  >
                    View file on GitHub
                  </button>
                )}
              </div>
              <FilePatch
                key={`${file.filename}:${head}`}
                file={file}
                split={split}
              />
            </>
          )}
        </div>
      </div>
    </section>
  );
}
