// Markdown from agents. Raw HTML is disabled, so the output is safe to inject. Code blocks get a copy
// button and finished assistant shell snippets can run in a new session-folder terminal.
// Finished messages are memoized; only the streaming one renders again.
//
// Inside a session, inline code naming a file of the session's folder (`hf_file_system.py:407`)
// becomes a link to the code viewer (CodeLinks context). The code viewer's Markdown preview resolves
// relative images and links to files of the same source (`links` prop). Verified commit IDs in
// prose and inline code link to their revision in the session project, even after worktree cleanup.

import { installFileCitations, type FileCitations } from "@/lib/fileCitations";
import { openExternal } from "@/lib/platform";
import MarkdownIt from "markdown-it";
import { useQueries } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { commitCandidates, installCommitLinks, type CommitLinks } from "@/lib/commitLinks";
import { createContext, memo, useContext, useMemo, type MouseEvent } from "react";
import { copyText } from "@/lib/clipboard";
import { useOpenTerminal } from "@/features/terminals/lib";
import { toast } from "./Toast";

const md = new MarkdownIt({ html: false, linkify: true, breaks: false });

/** Opens session file references and repository-verified commits in the code viewer. */
export type CodeLinks = FileCitations & {
  resolve: (raw: string) => { path: string; line: number | null } | null;
  open: (path: string, line: number | null) => void;
  commits?: { root: string; href: (sha: string) => string; open: (sha: string) => void };
};
export const CodeLinksContext = createContext<CodeLinks | null>(null);
export const ShellCodeContext = createContext<string | null>(null);

type Links = { image: (src: string) => string | null; link: (href: string) => boolean };
type Env = { codeLinks?: CodeLinks | null; links?: Links; runShell?: boolean; commits?: CommitLinks };
const SHELL_LANGUAGES = new Set(["sh", "bash", "zsh", "shell", "fish", "powershell", "pwsh"]);

const defaultFence = md.renderer.rules.fence!;
md.renderer.rules.fence = (tokens, idx, options, env, self) => {
  const html = defaultFence(tokens, idx, options, env, self);
  const language = tokens[idx].info.trim().split(/\s+/, 1)[0].toLowerCase();
  const run = (env as Env | undefined)?.runShell && SHELL_LANGUAGES.has(language) && tokens[idx].content.trim()
    ? `<button class="lp-copy" data-run="${idx}" data-language="${language}" title="Run this snippet in a new terminal in the session folder">Run in terminal</button>` : "";
  return `<div class="lp-code">${html}<div class="lp-code-actions">${run}<button class="lp-copy" data-copy="${idx}" title="Copy code">Copy</button></div></div>`;
};
md.renderer.rules.code_inline = (tokens, idx, _options, env) => {
  const content = tokens[idx].content;
  const hit = (env as Env | undefined)?.codeLinks?.resolve(content);
  const text = md.utils.escapeHtml(content);
  if (!hit) return `<code>${text}</code>`;
  return `<code class="lp-path" data-path="${md.utils.escapeHtml(hit.path)}" data-line="${hit.line ?? ""}" title="Open in Files">${text}</code>`;
};
const defaultImage = md.renderer.rules.image!;
md.renderer.rules.image = (tokens, idx, options, env, self) => {
  const src = tokens[idx].attrGet("src");
  const local = src ? (env as Env | undefined)?.links?.image(String(src)) : null;
  if (local) tokens[idx].attrSet("src", local);
  return defaultImage(tokens, idx, options, env, self);
};
// Link clicks are handled below so session files can open in the code viewer.
md.renderer.rules.link_open = (tokens, idx, options, _env, self) => {
  tokens[idx].attrSet("data-external", "1");
  return self.renderToken(tokens, idx, options);
};

installFileCitations(md);
installCommitLinks(md);

export const Markdown = memo(function Markdown({ text, className = "", links, runShell = false }: { text: string; className?: string; links?: Links; runShell?: boolean }) {
  const codeLinks = useContext(CodeLinksContext);
  const cwd = useContext(ShellCodeContext);
  const terminal = useOpenTerminal();
  const candidates = useMemo(() => codeLinks?.commits ? commitCandidates(md.parse(text, {})) : [], [text, codeLinks?.commits]);
  const batches = useMemo(() => {
    const chunks: string[][] = [];
    for (let i = 0; i < candidates.length; i += 64) chunks.push(candidates.slice(i, i + 64));
    return chunks;
  }, [candidates]);
  const resolved = useQueries({ queries: batches.map((oids) => ({
    queryKey: ["code-commit-links", codeLinks?.commits?.root, oids],
    queryFn: () => api.get<Record<string, string>>(`/api/code/resolve-commits?root=${encodeURIComponent(codeLinks!.commits!.root)}&oids=${oids.join(",")}`),
    staleTime: 60_000,
    retry: false,
  })) });
  const commits: CommitLinks = {};
  for (const result of resolved) for (const [oid, sha] of Object.entries(result.data ?? {})) {
    commits[oid] = { sha, href: codeLinks!.commits!.href(sha) };
  }
  // A stable signature keeps finished messages memoized when unrelated queries update.
  const commitSignature = JSON.stringify(commits);
  const { html, codes } = useMemo(() => {
    const env: Env = { codeLinks, links, runShell: runShell && !!cwd, commits: Object.keys(commits).length ? commits : undefined };
    const tokens = md.parse(text, env);
    const codes: Record<string, string> = {};
    tokens.forEach((t, i) => {
      if (t.type === "fence") codes[i] = t.content;
    });
    return { html: md.renderer.render(tokens, md.options, env), codes };
  }, [text, codeLinks, links, runShell, cwd, commitSignature]);

  const onClick = (e: MouseEvent<HTMLDivElement>) => {
    const target = e.target as HTMLElement;
    const run = target.closest<HTMLButtonElement>("[data-run]");
    if (run && runShell && cwd) {
      const command = codes[run.dataset.run!];
      if (!command?.trim() || run.disabled) return;
      run.disabled = true;
      void terminal.shell(cwd, command, run.dataset.language).finally(() => { run.disabled = false; });
      return;
    }
    const copy = target.closest<HTMLElement>("[data-copy]");
    if (copy) {
      void copyText(codes[copy.dataset.copy!] ?? "").then(() => toast("Code copied"));
      return;
    }
    const commit = target.closest<HTMLAnchorElement>("a[data-commit]");
    if (commit && codeLinks?.commits) {
      if (e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
      e.preventDefault();
      codeLinks.commits.open(commit.dataset.commit!);
      return;
    }
    const path = target.closest<HTMLElement>("[data-path]");
    if (path && codeLinks) {
      if (path.tagName === "A") {
        if (e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
        e.preventDefault();
      }
      codeLinks.open(path.dataset.path!, path.dataset.line ? Number(path.dataset.line) : null);
      return;
    }
    const a = target.closest<HTMLAnchorElement>("a[data-external]");
    if (a) {
      e.preventDefault();
      const href = a.getAttribute("href") ?? "";
      if (links?.link(href)) return;
      if (/^https?:\/\//i.test(href)) {
        openExternal(href);
        return;
      }
      if (href.startsWith("#")) return;
      let file = href.split(/[?#]/, 1)[0];
      try { file = decodeURIComponent(file); } catch { /* Keep the original path. */ }
      const hit = codeLinks?.resolve(file);
      if (hit) codeLinks!.open(hit.path, hit.line);
      else if (codeLinks) toast("This file is not in the agent's folder");
    }
  };
  return <div className={`lp-md ${className}`} onClick={onClick} dangerouslySetInnerHTML={{ __html: html }} />;
});
