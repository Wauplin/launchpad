// Markdown from agents. Raw HTML is disabled, so the output is safe to inject. Code blocks get a copy
// button. Finished messages are memoized; only the streaming one renders again.
//
// Inside a session, inline code naming a file of the session's folder (`hf_file_system.py:407`)
// becomes a link to the code viewer (CodeLinks context). The code viewer's Markdown preview resolves
// relative images and links to files of the same source (`links` prop).

import { api } from "@/lib/api";
import MarkdownIt from "markdown-it";
import { createContext, memo, useContext, useMemo, type MouseEvent } from "react";
import { copyText } from "@/lib/clipboard";
import { toast, toastError } from "./Toast";

const md = new MarkdownIt({ html: false, linkify: true, breaks: false });

/** Resolves `path:line` references of a session to its files, and opens them in the code viewer. */
export type CodeLinks = { resolve: (raw: string) => { path: string; line: number | null } | null; open: (path: string, line: number | null) => void };
export const CodeLinksContext = createContext<CodeLinks | null>(null);

type Links = { image: (src: string) => string | null; link: (href: string) => boolean };
type Env = { codeLinks?: CodeLinks | null; links?: Links };

const defaultFence = md.renderer.rules.fence!;
md.renderer.rules.fence = (tokens, idx, options, env, self) => {
  const html = defaultFence(tokens, idx, options, env, self);
  return `<div class="lp-code">${html}<button class="lp-copy" data-copy="${idx}" title="Copy code">Copy</button></div>`;
};
md.renderer.rules.code_inline = (tokens, idx, _options, env) => {
  const content = tokens[idx].content;
  const hit = (env as Env | undefined)?.codeLinks?.resolve(content);
  const text = md.utils.escapeHtml(content);
  if (!hit) return `<code>${text}</code>`;
  return `<code class="lp-path" data-path="${md.utils.escapeHtml(hit.path)}" data-line="${hit.line ?? ""}" title="Open in the code viewer">${text}</code>`;
};
const defaultImage = md.renderer.rules.image!;
md.renderer.rules.image = (tokens, idx, options, env, self) => {
  const src = tokens[idx].attrGet("src");
  const local = src ? (env as Env | undefined)?.links?.image(String(src)) : null;
  if (local) tokens[idx].attrSet("src", local);
  return defaultImage(tokens, idx, options, env, self);
};
// Links open in the browser through the daemon (see onClick below).
md.renderer.rules.link_open = (tokens, idx, options, _env, self) => {
  tokens[idx].attrSet("data-external", "1");
  return self.renderToken(tokens, idx, options);
};

export const Markdown = memo(function Markdown({ text, className = "", links }: { text: string; className?: string; links?: Links }) {
  const codeLinks = useContext(CodeLinksContext);
  const { html, codes } = useMemo(() => {
    const env: Env = { codeLinks, links };
    const tokens = md.parse(text, env);
    const codes: Record<string, string> = {};
    tokens.forEach((t, i) => {
      if (t.type === "fence") codes[i] = t.content;
    });
    return { html: md.renderer.render(tokens, md.options, env), codes };
  }, [text, codeLinks, links]);

  const onClick = (e: MouseEvent<HTMLDivElement>) => {
    const target = e.target as HTMLElement;
    const copy = target.closest<HTMLElement>("[data-copy]");
    if (copy) {
      void copyText(codes[copy.dataset.copy!] ?? "").then(() => toast("Code copied"));
      return;
    }
    const path = target.closest<HTMLElement>("[data-path]");
    if (path && codeLinks) {
      codeLinks.open(path.dataset.path!, path.dataset.line ? Number(path.dataset.line) : null);
      return;
    }
    const a = target.closest<HTMLAnchorElement>("a[data-external]");
    if (a) {
      e.preventDefault();
      if (links?.link(a.getAttribute("href") ?? "")) return;
      void api.post("/api/open", { url: a.href }).catch(toastError);
    }
  };
  return <div className={`lp-md ${className}`} onClick={onClick} dangerouslySetInnerHTML={{ __html: html }} />;
});
