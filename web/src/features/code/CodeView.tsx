// Code viewer (sketch S7, ARCHITECTURE.md section 8.1): one source at a time (a working tree or a git
// revision), a file tree, tabs, colored code with changed-line bars, find in file (Ctrl+F) and
// search in the project. Selected lines become agent context. Read only.

import { useQueryClient } from "@tanstack/react-query";
import { useNavigate, useParams, useSearch } from "@tanstack/react-router";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { CodeFile } from "@/gen/CodeFile";
import type { CommitInfo } from "@/gen/CommitInfo";
import type { FileRef } from "@/gen/FileRef";
import type { Project } from "@/gen/Project";
import type { SessionEvent } from "@/gen/SessionEvent";
import type { Session } from "@/gen/Session";
import { HarnessMark, useHarnessName } from "@/components/HarnessMark";
import { Icon } from "@/components/Icon";
import { Markdown } from "@/components/Markdown";
import DOMPurify from "dompurify";
import { MenuItem, Popover } from "@/components/Popover";
import { toast } from "@/components/Toast";
import { api } from "@/lib/api";
import { copyText } from "@/lib/clipboard";
import { formatAge } from "@/lib/format";
import { useProject, useSessions } from "@/lib/queries";
import { isDone, isRunning } from "@/lib/sessionStatus";
import { onEvent } from "@/lib/ws";
import { useComposer } from "@/stores/composer";
import { useLayout } from "@/stores/layout";
import { useMessageDrafts } from "@/stores/messages";
import { CodeLines, type CodeLinesHandle, type Selection } from "./CodeLines";
import { useTheme, type Theme } from "@/stores/theme";
import { cachedTokens, highlight, type Tokens } from "./highlight";
import { basename, dirname, findAll, githubUrl, lineRef, matcher, rawUrl, useCodeFile, useCodeFiles, useCodeSources, useCommit, type FindOpts } from "./lib";
import { FilesPanel, FindToggles, SearchPanel } from "./SidePanel";
import { sourceKey, useCode, type CodeSearch } from "./store";
import { useOpenTerminal } from "@/features/terminals/lib";

const MAX_COLORS_BYTES = 1024 * 1024;
const MAX_COLORS_LINES = 20_000;

const sizeText = (n: number) => (n < 1024 ? `${n} B` : n < 1024 * 1024 ? `${(n / 1024).toFixed(1)} KB` : `${(n / 1024 / 1024).toFixed(1)} MB`);

export function CodeView() {
  const { id } = useParams({ strict: false }) as { id: string };
  const search = useSearch({ strict: false }) as CodeSearch;
  const project = useProject(id);
  const navigate = useNavigate();
  const last = useCode((s) => s.last[id]);
  const setLast = useCode((s) => s.setLast);

  // Without a source, go back to where you were in this project, or to its checkout.
  useEffect(() => {
    if (!project || search.root) return;
    const to = last?.root ? last : { root: project.path };
    void navigate({ to: "/projects/$id/code", params: { id }, search: to, replace: true });
  }, [project, search.root, last, id, navigate]);
  useEffect(() => {
    if (search.root) setLast(id, { root: search.root, rev: search.rev, path: search.path });
  }, [id, search.root, search.rev, search.path, setLast]);

  if (!project) return <div className="p-6 text-ink-3">This project is not registered.</div>;
  if (!search.root) return null;
  return <Viewer key={sourceKey(search.root, search.rev)} project={project} root={search.root} rev={search.rev ?? null} path={search.path ?? null} line={search.line ?? null} session={search.session ?? null} />;
}

function Viewer({ project, root, rev, path, line, session }: { project: Project; root: string; rev: string | null; path: string | null; line: number | null; session: string | null }) {
  const navigate = useNavigate();
  const qc = useQueryClient();
  const skey = sourceKey(root, rev);
  const list = useCodeFiles(root, rev, { always: true });
  const tabs = useCode((s) => s.sources[skey]?.tabs) ?? [];
  const openTab = useCode((s) => s.openTab);
  const closeTab = useCode((s) => s.closeTab);
  const [panel, setPanel] = useState<"files" | "search">("files");
  const openTerminal = useOpenTerminal();
  const [searchState, setSearchState] = useState<{ q: string; opts: FindOpts }>({ q: "", opts: { case: false, word: false, regex: false } });

  const go = useCallback(
    (s: CodeSearch) => void navigate({ to: "/projects/$id/code", params: { id: project.id }, search: { root, rev: rev ?? undefined, session: session ?? undefined, ...s }, replace: true }),
    [navigate, project.id, root, rev, session],
  );
  const open = useCallback((p: string, l?: number) => go({ path: p, line: l }), [go]);

  useEffect(() => {
    if (path) openTab(skey, path);
  }, [path, skey, openTab]);

  // First visit of a source: show its README.
  useEffect(() => {
    if (path || tabs.length || !list.data) return;
    const readme = list.data.files.find((f) => /^readme(\.md|\.rst|\.txt)?$/i.test(f));
    if (readme) open(readme);
  }, [list.data]);

  // Live updates: a session that edits a file of this working tree reloads it in place.
  useEffect(() => {
    if (rev) return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const off = onEvent((ev) => {
      if (!ev.topic.startsWith("session/")) return;
      const e = ev.payload as SessionEvent;
      if (e.body.kind !== "file_change") return;
      const s = qc.getQueryData<Session[]>(["sessions"])?.find((x) => x.id === e.session_id);
      const abs = e.body.path.startsWith("/") ? e.body.path : s ? `${s.cwd}/${e.body.path}` : null;
      if (!abs?.startsWith(`${root}/`)) return;
      void qc.invalidateQueries({ queryKey: ["code-file", root, null, abs.slice(root.length + 1)] });
      clearTimeout(timer);
      timer = setTimeout(() => void qc.invalidateQueries({ queryKey: ["code-files", root, null] }), 1000);
    });
    return () => {
      off();
      clearTimeout(timer);
    };
  }, [qc, root, rev]);

  return (
    <div className="grid h-full grid-cols-[284px_minmax(0,1fr)]">
      <aside className="flex min-h-0 min-w-0 flex-col border-r border-line bg-paper">
        <div className="grid grid-cols-[minmax(0,1fr)] gap-2 px-2.5 pt-2.5 pb-1">
          {session && <BackToSession id={session} />}
          <div className="flex min-w-0 gap-1.5">
            <div className="min-w-0 flex-1"><SourcePicker project={project} root={root} rev={rev} path={path} /></div>
            <button className="btn h-[34px]!" title={`Open a shell in ${root}`} onClick={() => void openTerminal.shell(root)}>
              <Icon name="term" size={14} />
              Terminal
            </button>
          </div>
          <span className="inline-flex gap-0.5 rounded-lg bg-sunk p-0.5">
            {(["files", "search"] as const).map((p) => (
              <button
                key={p}
                onClick={() => setPanel(p)}
                className={`h-[26px] flex-1 rounded-md text-[12.5px] ${panel === p ? "bg-paper font-semibold text-ink shadow-[0_1px_2px_rgba(23,32,43,.12),inset_0_0_0_1px_var(--color-line)]" : "text-ink-2"}`}
              >
                {p === "files" ? "Files" : "Search"}
              </button>
            ))}
          </span>
        </div>
        {list.isError && <div className="px-3 py-2 text-xs text-gh-closed">{(list.error as Error).message}</div>}
        {panel === "files" ? (
          <FilesPanel list={list.data} skey={skey} active={path} onOpen={open} />
        ) : (
          <SearchPanel root={root} rev={rev} onOpen={open} state={searchState} setState={setSearchState} />
        )}
      </aside>
      <div className="flex min-h-0 min-w-0 flex-col">
        <div className="flex h-[34px] flex-none items-stretch overflow-x-auto border-b border-line bg-chrome">
          {tabs.map((t) => (
            <div
              key={t}
              className={`group flex flex-none items-center gap-1.5 border-r border-line pr-1 pl-3 text-[12.5px] ${t === path ? "bg-paper font-semibold text-ink shadow-[inset_0_2px_0_var(--color-teal)]" : "text-ink-2 hover:bg-sunk"} ${list.data?.changes[t] ? "text-attn!" : ""}`}
            >
              <button className="flex h-full items-center gap-1.5" title={t} onClick={() => open(t)}>
                <Icon name="file" size={12} className="text-ink-3" />
                {basename(t)}
                {tabHint(t, tabs) && <span className="text-[11px] font-normal text-ink-3">{tabHint(t, tabs)}</span>}
              </button>
              <button
                className={`iconbtn size-5! ${t === path ? "" : "opacity-0 group-hover:opacity-100"}`}
                title="Close tab"
                onClick={() => {
                  const next = closeTab(skey, t);
                  if (t === path) go({ path: next ?? undefined });
                }}
              >
                <Icon name="x" size={11} />
              </button>
            </div>
          ))}
        </div>
        {path ? (
          <FileView key={`${skey}\n${path}`} project={project} root={root} rev={rev} path={path} line={line} letter={list.data?.changes[path]} />
        ) : (
          <div className="grid flex-1 place-items-center text-ink-3">{list.data?.files.length === 0 ? "This source has no file." : "Pick a file in the tree, or search the project."}</div>
        )}
      </div>
    </div>
  );
}

/** For tabs with the same file name: the shortest end of the folder that tells them apart. */
function tabHint(t: string, tabs: string[]): string | null {
  const same = tabs.filter((o) => o !== t && basename(o) === basename(t));
  if (!same.length) return null;
  const dirs = dirname(t).split("/");
  for (let k = 1; k <= dirs.length; k++) {
    const end = dirs.slice(-k).join("/");
    if (!same.some((o) => dirname(o).split("/").slice(-k).join("/") === end)) return end;
  }
  return dirname(t) || "/";
}

/** Shown when the viewer was opened from an agent session. */
function BackToSession({ id }: { id: string }) {
  const navigate = useNavigate();
  const title = useSessions().data?.find((s) => s.id === id)?.title;
  if (!title) return null;
  return <button className="truncate text-left text-xs text-teal-ink hover:underline" title={`Back to the session "${title}"`} onClick={() => void navigate({ to: "/agents", search: { session: id } })}>← Back to session · {title}</button>;
}

// ---------------------------------------------------------------- source picker

function SourcePicker({ project, root, rev, path }: { project: Project; root: string; rev: string | null; path: string | null }) {
  const harnessName = useHarnessName();
  const navigate = useNavigate();
  const sources = useCodeSources(project.id).data;
  const sessions = useSessions().data ?? [];
  const [filter, setFilter] = useState("");
  const pick = (r: string, v: string | null) => void navigate({ to: "/projects/$id/code", params: { id: project.id }, search: { root: r, rev: v ?? undefined, path: path ?? undefined } });
  const worktree = sources?.worktrees.find((w) => w.path === root);
  const current = rev ? sources?.refs.find((r) => r.name === rev) : null;
  const f = filter.trim().toLowerCase();
  const refs = (sources?.refs ?? []).filter((r) => !f || r.label.toLowerCase().includes(f)).slice(0, 60);
  const label = rev ? (current?.label ?? rev) : worktree?.main ? "Checkout" : (worktree?.branch ?? basename(root));
  const detail = (wt: string) => {
    const here = sessions.filter((s) => s.cwd === wt && (isRunning(s) || isDone(s)));
    const s = here.find(isRunning) ?? here[0];
    if (s) return `${harnessName(s.harness)} ${isRunning(s) ? "is working here" : "is done here"}`;
    return wt.replace(/^\/home\/[^/]+/, "~");
  };
  return (
    <Popover
      width={340}
      trigger={({ toggle }) => (
        <button onClick={toggle} className="flex h-[34px] w-full min-w-0 items-center gap-2 rounded-[7px] border border-line bg-paper px-2.5 text-left text-[12.5px] hover:border-line" title={rev ? `${rev} in ${root}` : root}>
          <Icon name={rev ? "branch" : "folder"} size={14} className="text-ink-3" />
          <span className="min-w-0 flex-1 truncate">
            <b className={rev || !worktree?.main ? "font-mono text-xs" : "font-semibold"}>{label}</b>
            {!rev && worktree?.main && worktree.branch && <span className="font-mono text-xs text-ink-3"> · {worktree.branch}</span>}
            {rev && <span className="text-xs text-ink-3"> · read from git</span>}
          </span>
          <Icon name="chev" size={14} className="flex-none text-ink-3" />
        </button>
      )}
    >
      {(close) => (
        <>
          <input className="field-input mb-1 h-7! text-xs" placeholder="Branch, tag or commit" value={filter} onChange={(e) => setFilter(e.target.value)} />
          {!f && <div className="seclabel px-2 pt-1 pb-0.5">Working trees</div>}
          {!f &&
            (sources?.worktrees ?? [{ path: project.path, branch: null, main: true }]).map((w) => {
              const s = sessions.find((x) => x.cwd === w.path && isRunning(x));
              return (
                <MenuItem
                  key={w.path}
                  icon={s ? <HarnessMark harness={s.harness} size={14} /> : <Icon name="folder" size={13} />}
                  label={
                    w.main ? (
                      <>
                        <b className="font-semibold">Checkout</b>
                        {w.branch && <span className="font-mono text-xs"> · {w.branch}</span>}
                      </>
                    ) : (
                      <b className="font-mono text-xs font-semibold">{w.branch ?? basename(w.path)}</b>
                    )
                  }
                  detail={detail(w.path)}
                  active={!rev && w.path === root}
                  onClick={() => (pick(w.path, null), close())}
                />
              );
            })}
          {sources?.git !== false && <div className="seclabel px-2 pt-2 pb-0.5">Branches, tags and pull requests</div>}
          {refs.map((r) => (
            <MenuItem
              key={r.name}
              icon={<Icon name={r.kind === "pr" ? "pr" : r.kind === "tag" ? "commit" : "branch"} size={13} />}
              label={<span className="font-mono text-xs">{r.label}</span>}
              detail={`${r.kind === "branch" ? "Local branch" : r.kind === "remote" ? "Remote branch" : r.kind === "tag" ? "Tag" : "PR head, fetched for its diff"}${r.date ? ` · ${formatAge(r.date)} ago` : ""}`}
              active={rev === r.name}
              onClick={() => (pick(project.path, r.name), close())}
            />
          ))}
          {f && !refs.some((r) => r.label.toLowerCase() === f) && (
            <MenuItem icon={<Icon name="commit" size={13} />} label={<span className="font-mono text-xs">{filter.trim()}</span>} detail="Open this revision (a commit, or any git revision)" onClick={() => (pick(project.path, filter.trim()), close())} />
          )}
          {f && refs.length === 0 && <div className="px-2 py-1 text-xs text-ink-3">No branch or tag matches.</div>}
        </>
      )}
    </Popover>
  );
}

// ---------------------------------------------------------------- one file

function splitLines(text: string): string[] {
  const lines = text.split("\n");
  if (lines.length > 1 && lines[lines.length - 1] === "") lines.pop();
  return lines.map((l) => (l.endsWith("\r") ? l.slice(0, -1) : l));
}

function FileView({ project, root, rev, path, line, letter }: { project: Project; root: string; rev: string | null; path: string; line: number | null; letter: string | undefined }) {
  const qc = useQueryClient();
  const q = useCodeFile(root, rev, path);
  const commit = useCommit(root, rev).data;
  const f = q.data;
  const skey = sourceKey(root, rev);
  const [preview, setPreview] = useState(false);
  const [findOpen, setFindOpen] = useState(false);
  const findRef = useRef<HTMLInputElement>(null);
  const lines = useRef<CodeLinesHandle>(null);
  const markdown = f?.language === "markdown";
  const html = f?.language === "html";
  const isText = f?.kind === "text" && f.text !== null;

  const openFind = useCallback(() => {
    setPreview(false);
    setFindOpen(true);
    requestAnimationFrame(() => {
      findRef.current?.focus();
      findRef.current?.select();
    });
  }, []);
  // Ctrl+F while the viewer is on screen: the only keyboard shortcut in Launchpad (decision 22).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // A focused terminal gets every key, Ctrl+F included.
      if (e.target instanceof Element && e.target.closest(".xterm")) return;
      if (e.ctrlKey && !e.altKey && !e.shiftKey && e.key.toLowerCase() === "f") {
        e.preventDefault();
        if (isText) openFind();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [isText, openFind]);

  const reload = () => {
    void qc.invalidateQueries({ queryKey: ["code-file", root, rev, path] });
    void qc.invalidateQueries({ queryKey: ["code-files", root, rev] });
    void qc.invalidateQueries({ queryKey: ["code-commit", root, rev] });
  };
  const ghFile = githubUrl(commit, path);
  const dirtyReason = f?.dirty ? "This file has uncommitted changes, so GitHub shows other lines" : null;
  const ghTitle = !commit ? "Checking GitHub…" : !ghFile ? "This commit is not on GitHub" : (dirtyReason ?? "Open on GitHub at this commit");

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex min-h-[42px] flex-none flex-wrap items-center gap-x-3 gap-y-1 border-b border-line-2 px-3.5 py-1.5">
        <span className="min-w-0 truncate font-mono text-[12.5px] text-ink-2" title={path}>
          {dirname(path) && <>{dirname(path).split("/").join(" / ")} / </>}
          <b className="font-semibold text-ink">{basename(path)}</b>
        </span>
        {f && <FileMeta f={f} letter={letter} />}
        <span className="ml-auto flex items-center gap-1">
          {isText && (
            <button className={`btn btn-sm ${findOpen ? "border-teal! bg-teal-soft!" : ""}`} title="Find in this file (Ctrl+F)" onClick={openFind}>
              <Icon name="search" size={12} />
              Find <span className="rounded border border-line px-1 font-mono text-[10px] text-ink-3">Ctrl+F</span>
            </button>
          )}
          {(markdown || html) && isText && (
            <button className="btn btn-sm" title={preview ? "Show the source" : `Show the rendered ${html ? "HTML" : "Markdown"}`} onClick={() => setPreview((p) => !p)}>
              <Icon name={preview ? "file" : "eye"} size={12} />
              {preview ? "Source" : "Preview"}
            </button>
          )}
          <button className="iconbtn" title="Copy path" onClick={() => void copyText(path).then(() => toast("Path copied"))}>
            <Icon name="copy" size={14} />
          </button>
          <button className="iconbtn disabled:opacity-40" disabled={!ghFile} title={ghTitle} onClick={() => ghFile && void api.post("/api/open", { url: ghFile })}>
            <Icon name="ext" size={14} />
          </button>
          <button className="iconbtn" title="Reload the file and the file list" onClick={reload}>
            <Icon name="refresh" size={14} />
          </button>
        </span>
      </div>
      {q.isError ? (
        <div className="grid flex-1 place-items-center text-ink-3">{(q.error as Error).message}</div>
      ) : !f ? (
        <div className="grid flex-1 place-items-center text-ink-3">Loading…</div>
      ) : f.kind === "image" ? (
        <div className="cv-check flex min-h-0 flex-1 items-center justify-center p-6">
          <img src={rawUrl(root, rev, path)} alt={path} className="max-h-full max-w-full border border-line-2 object-contain" />
        </div>
      ) : f.kind === "binary" || f.kind === "too_large" ? (
        <div className="grid flex-1 place-items-center text-center text-ink-2">
          <div className="grid justify-items-center gap-2">
            {f.kind === "binary" ? `Binary file · ${sizeText(f.size)}` : `This file is ${sizeText(f.size)}: too large to show here.`}
          </div>
        </div>
      ) : preview && html ? (
        <HtmlPreview text={f.text ?? ""} path={path} />
      ) : preview && markdown ? (
        <MarkdownPreview text={f.text ?? ""} root={root} rev={rev} path={path} projectId={project.id} />
      ) : (
        <TextView
          f={f}
          fileKey={`${skey}\n${path}`}
          project={project}
          root={root}
          rev={rev}
          path={path}
          line={line}
          commit={commit}
          handle={lines}
          findOpen={findOpen}
          setFindOpen={setFindOpen}
          findRef={findRef}
        />
      )}
    </div>
  );
}

function FileMeta({ f, letter }: { f: CodeFile; letter: string | undefined }) {
  const parts: React.ReactNode[] = [f.language_name ?? (f.kind === "text" ? "Plain text" : f.kind === "image" ? "Image" : "Binary")];
  if (f.text !== null) {
    const n = splitLines(f.text).length;
    parts.push(`${n.toLocaleString()} line${n === 1 ? "" : "s"}`);
  }
  const since = f.base ? ` since ${f.base.replace(/^origin\//, "")}` : "";
  const l = f.letter ?? letter;
  if (l === "U") parts.push("untracked");
  else if (f.added || f.removed)
    parts.push(
      <span key="d">
        <span className="font-semibold text-gh-open">+{f.added}</span> <span className="font-semibold text-gh-closed">−{f.removed}</span>
        {since}
        {l === "A" && " (new file)"}
      </span>,
    );
  if (f.kind === "text" && (f.size > MAX_COLORS_BYTES || (f.text && splitLines(f.text).length > MAX_COLORS_LINES))) parts.push("no colors (large file)");
  return (
    <span className="text-xs whitespace-nowrap text-ink-3">
      {parts.map((p, i) => (
        <span key={i}>
          {i > 0 && " · "}
          {p}
        </span>
      ))}
    </span>
  );
}

function HtmlPreview({ text, path }: { text: string; path: string }) {
  const document = useMemo(() => {
    // Keep document styles, but never let file content run code or navigate to daemon URLs.
    const safe = DOMPurify.sanitize(text, {
      WHOLE_DOCUMENT: true,
      ADD_TAGS: ["style"],
      FORBID_TAGS: ["base", "meta", "link"],
      FORBID_ATTR: ["href"],
    });
    const policy = `<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; img-src data:; font-src data:; form-action 'none'; base-uri 'none'">`;
    return safe.replace("<head>", `<head>${policy}`);
  }, [text]);
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="border-b border-line-2 px-3.5 py-2 text-xs text-ink-3">Static HTML preview · Inline styles and embedded images only. Scripts, links and external files are disabled.</div>
      <iframe title={`HTML preview: ${path}`} sandbox="" referrerPolicy="no-referrer" srcDoc={document} className="min-h-0 w-full flex-1 border-0 bg-white" />
    </div>
  );
}

function MarkdownPreview({ text, root, rev, path, projectId }: { text: string; root: string; rev: string | null; path: string; projectId: string }) {
  const navigate = useNavigate();
  // Relative images and links point to files of the same source.
  const resolve = useCallback(
    (href: string) => {
      if (/^[a-z]+:|^#|^\/\//i.test(href)) return null;
      const parts = (href.startsWith("/") ? href.slice(1) : `${dirname(path)}/${href}`).split("#")[0].split("/");
      const out: string[] = [];
      for (const p of parts) {
        if (p === "..") out.pop();
        else if (p && p !== ".") out.push(p);
      }
      return out.join("/") || null;
    },
    [path],
  );
  const links = useMemo(
    () => ({
      image: (src: string) => {
        const p = resolve(src);
        return p ? rawUrl(root, rev, p) : null;
      },
      link: (href: string) => {
        const p = resolve(href);
        if (!p) return false;
        void navigate({ to: "/projects/$id/code", params: { id: projectId }, search: { root, rev: rev ?? undefined, path: p }, replace: true });
        return true;
      },
    }),
    [resolve, root, rev, navigate, projectId],
  );
  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto max-w-[860px] px-8 py-6">
        <Markdown text={text} className="lp-md-doc" links={links} />
      </div>
    </div>
  );
}

// ---------------------------------------------------------------- text, find, selection

function TextView({
  f,
  fileKey,
  project,
  root,
  rev,
  path,
  line,
  commit,
  handle,
  findOpen,
  setFindOpen,
  findRef,
}: {
  f: CodeFile;
  fileKey: string;
  project: Project;
  root: string;
  rev: string | null;
  path: string;
  line: number | null;
  commit: CommitInfo | undefined;
  handle: React.RefObject<CodeLinesHandle | null>;
  findOpen: boolean;
  setFindOpen: (v: boolean) => void;
  findRef: React.RefObject<HTMLInputElement | null>;
}) {
  const text = f.text ?? "";
  const lines = useMemo(() => splitLines(text), [text]);
  const skey = sourceKey(root, rev);
  const setPos = useCode((s) => s.setPos);
  const savedTop = useCode.getState().sources[skey]?.pos[path];
  const [selection, setSelection] = useState<Selection | null>(line ? { start: line, end: line } : null);

  // Colors: plain text first, Shiki when the worker answers. None for very large files.
  const theme = useTheme((s) => s.theme);
  const colors = !!f.language && f.size <= MAX_COLORS_BYTES && lines.length <= MAX_COLORS_LINES;
  const [tokens, setTokens] = useState<{ text: string; theme: Theme; tokens: Tokens } | null>(() => {
    const t = colors ? cachedTokens(fileKey, text, theme) : null;
    return t ? { text, theme, tokens: t } : null;
  });
  useEffect(() => {
    if (!colors || !f.language || (tokens?.text === text && tokens.theme === theme)) return;
    let alive = true;
    void highlight(fileKey, text, f.language, theme).then((t) => alive && t && setTokens({ text, theme, tokens: t }));
    return () => {
      alive = false;
    };
  }, [colors, f.language, fileKey, text, tokens?.text, tokens?.theme, theme]);

  // Find in file: the whole text, in memory.
  const [query, setQuery] = useState("");
  const [opts, setOpts] = useState<FindOpts>({ case: false, word: false, regex: false });
  const [index, setIndex] = useState(0);
  const re = useMemo(() => (findOpen ? matcher(query, opts) : null), [findOpen, query, opts]);
  const matches = useMemo(() => (re instanceof RegExp ? findAll(lines, re) : []), [re, lines]);
  const byLine = useMemo(() => {
    const m = new Map<number, [number, number][]>();
    for (const x of matches) {
      const list = m.get(x.line);
      if (list) list.push([x.start, x.end]);
      else m.set(x.line, [[x.start, x.end]]);
    }
    return m;
  }, [matches]);
  const cur = matches.length ? matches[Math.min(index, matches.length - 1)] : null;
  const show = useCallback(
    (i: number) => {
      const m = matches[i];
      if (!m) return;
      setIndex(i);
      handle.current?.scrollToLine(m.line + 1, "center");
      handle.current?.revealMatch();
    },
    [matches, handle],
  );
  // A new query starts at the first match from the top of the screen.
  useEffect(() => {
    if (!matches.length) return;
    const top = (handle.current?.topLine() ?? 1) - 1;
    const i = matches.findIndex((m) => m.line >= top);
    show(i < 0 ? 0 : i);
  }, [query, opts]);
  const step = (d: number) => matches.length && show((Math.min(index, matches.length - 1) + d + matches.length) % matches.length);

  // A new `line` in the address (a search result in the same file).
  const first = useRef(true);
  useEffect(() => {
    if (first.current) {
      first.current = false;
      return;
    }
    if (line) {
      setSelection({ start: line, end: line });
      handle.current?.scrollToLine(line, "center");
    }
  }, [line, handle]);

  const initialTop = useMemo(() => (line ? { line, align: "center" as const } : savedTop ? { line: savedTop, align: "start" as const } : null), [line, savedTop]);
  const onTop = useCallback((l: number) => setPos(skey, path, l), [setPos, skey, path]);

  return (
    <div className="relative flex min-h-0 flex-1 flex-col">
      {findOpen && (
        <div className="absolute top-2 right-5 z-20 flex items-center gap-1 rounded-lg border border-line bg-paper py-1 pr-1 pl-2 shadow-[0_8px_22px_-10px_rgba(23,32,43,.4)]">
          <Icon name="search" size={13} className="text-ink-3" />
          <input
            ref={findRef}
            className="h-6 w-56 bg-transparent text-[12.5px] outline-none"
            placeholder="Find in file"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              // The find field's keys (decision 22): Enter, Shift+Enter, Escape.
              if (e.key === "Enter") {
                e.preventDefault();
                step(e.shiftKey ? -1 : 1);
              } else if (e.key === "Escape") {
                e.preventDefault();
                setFindOpen(false);
              }
            }}
          />
          <span className={`min-w-16 text-right text-[11.5px] tabular-nums ${typeof re === "string" ? "text-gh-closed" : "text-ink-3"}`}>
            {typeof re === "string" ? "Invalid" : !query ? "" : matches.length ? `${Math.min(index, matches.length - 1) + 1} of ${matches.length >= 10_000 ? "10,000+" : matches.length}` : "No match"}
          </span>
          <FindToggles opts={opts} set={setOpts} />
          <span className="mx-0.5 h-4 w-px bg-line" />
          <button className="iconbtn" title="Previous match (Shift+Enter)" disabled={!matches.length} onClick={() => step(-1)}>
            <Icon name="up" size={13} />
          </button>
          <button className="iconbtn" title="Next match (Enter)" disabled={!matches.length} onClick={() => step(1)}>
            <Icon name="up" size={13} className="rotate-180" />
          </button>
          <button className="iconbtn" title="Close (Escape)" onClick={() => setFindOpen(false)}>
            <Icon name="x" size={13} />
          </button>
        </div>
      )}
      <CodeLines
        lines={lines}
        tokens={(tokens?.text === text && tokens.theme === theme) ? tokens.tokens : null}
        marks={f.marks}
        matchesByLine={byLine}
        current={cur ? { line: cur.line, start: cur.start } : null}
        selection={selection}
        onSelect={setSelection}
        initialTop={initialTop}
        onTop={onTop}
        handle={handle}
      />
      {selection && (
        <SelectionBar
          selection={selection}
          lines={lines}
          project={project}
          root={root}
          rev={rev}
          path={path}
          commit={commit}
          dirty={f.dirty}
          onClear={() => setSelection(null)}
        />
      )}
    </div>
  );
}

function SelectionBar({
  selection,
  lines,
  project,
  root,
  rev,
  path,
  commit,
  dirty,
  onClear,
}: {
  selection: Selection;
  lines: string[];
  project: Project;
  root: string;
  rev: string | null;
  path: string;
  commit: CommitInfo | undefined;
  dirty: boolean;
  onClear: () => void;
}) {
  const { start, end } = selection;
  const gh = githubUrl(commit, path, selection);
  const ghTitle = !gh ? "This commit is not on GitHub" : dirty ? "This file has uncommitted changes, so GitHub shows other lines" : "Copy a GitHub permalink to these lines";
  return (
    <div className="absolute right-5 bottom-4 z-20 flex max-w-[calc(100%-96px)] flex-wrap justify-end items-center gap-1.5 rounded-[10px] border border-line bg-paper px-2 py-1.5 whitespace-nowrap shadow-[0_10px_26px_-10px_rgba(23,32,43,.45)]">
      <span className="px-1 text-xs font-semibold">{end > start ? `Lines ${start}–${end}` : `Line ${start}`}</span>
      <button className="btn btn-sm" title="Copy the code" onClick={() => void copyText(lines.slice(start - 1, end).join("\n")).then(() => toast("Code copied"))}>
        <Icon name="copy" size={12} />
        Copy
      </button>
      <button className="btn btn-sm" title={lineRef(path, selection)} onClick={() => void copyText(lineRef(path, selection)).then(() => toast("Reference copied"))}>
        Copy reference
      </button>
      <button className="btn btn-sm" disabled={!gh || dirty} title={ghTitle} onClick={() => gh && void copyText(gh).then(() => toast("GitHub link copied"))}>
        <Icon name="ext" size={12} />
        GitHub link
      </button>
      <AddToAgent project={project} root={root} rev={rev} path={path} selection={selection} />
      <button className="iconbtn" title="Clear the selection" onClick={onClear}>
        <Icon name="x" size={13} />
      </button>
    </div>
  );
}

function AddToAgent({ project, root, rev, path, selection }: { project: Project; root: string; rev: string | null; path: string; selection: Selection }) {
  const navigate = useNavigate();
  const sessions = (useSessions().data ?? []).filter((s) => s.project_id === project.id && s.status !== "closed").slice(0, 6);
  const show = useLayout((s) => s.show);
  const append = useMessageDrafts((s) => s.append);
  const ref = lineRef(path, selection);
  const toComposer = () => {
    const file: FileRef = { root, rev, path, start_line: selection.start, end_line: selection.end };
    const target = useComposer.getState().spec.target;
    useComposer.getState().start({ projectId: target.kind === "none" ? project.id : undefined, files: [file] });
  };
  const toSession = (s: Session) => {
    // Relative to the session's folder when it reads the same working tree.
    const text = rev ? `${ref} (at ${rev})` : s.cwd === root ? ref : `${root}/${ref}`;
    append(s.id, text);
    show(s.id);
    void navigate({ to: "/agents" });
  };
  return (
    <Popover
      up
      width={320}
      trigger={({ toggle }) => (
        <button className="btn btn-sm btn-primary" title="Give these lines to an agent as context" onClick={toggle}>
          <Icon name="play" size={12} />
          Add to agent
          <Icon name="chev" size={12} />
        </button>
      )}
    >
      {(close) => (
        <>
          <MenuItem icon={<Icon name="plus" size={13} />} label={<b className="font-semibold">New agent</b>} detail="Opens the composer with these lines as context" onClick={() => (toComposer(), close())} />
          {sessions.map((s) => (
            <MenuItem
              key={s.id}
              icon={<HarnessMark harness={s.harness} size={14} />}
              label={<b className="font-semibold">{s.title}</b>}
              detail={`Adds the reference to its message box · ${isRunning(s) ? "Running" : isDone(s) ? "Done" : "Archived"}`}
              onClick={() => (toSession(s), close())}
            />
          ))}
        </>
      )}
    </Popover>
  );
}
