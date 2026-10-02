// Markdown from agents. Raw HTML is disabled, so the output is safe to inject. Code blocks get a copy
// button. Finished messages are memoized; only the streaming one renders again.

import MarkdownIt from "markdown-it";
import { memo, useMemo, type MouseEvent } from "react";
import { copyText } from "@/lib/clipboard";
import { toast } from "./Toast";

const md = new MarkdownIt({ html: false, linkify: true, breaks: false });

const defaultFence = md.renderer.rules.fence!;
md.renderer.rules.fence = (tokens, idx, options, env, self) => {
  const html = defaultFence(tokens, idx, options, env, self);
  return `<div class="lp-code">${html}<button class="lp-copy" data-copy="${idx}" title="Copy code">Copy</button></div>`;
};
// Links open in the browser through the daemon (see onClick below).
md.renderer.rules.link_open = (tokens, idx, options, _env, self) => {
  tokens[idx].attrSet("data-external", "1");
  return self.renderToken(tokens, idx, options);
};

export const Markdown = memo(function Markdown({ text, className = "" }: { text: string; className?: string }) {
  const { html, codes } = useMemo(() => {
    const tokens = md.parse(text, {});
    const codes: Record<string, string> = {};
    tokens.forEach((t, i) => {
      if (t.type === "fence") codes[i] = t.content;
    });
    return { html: md.renderer.render(tokens, md.options, {}), codes };
  }, [text]);

  const onClick = (e: MouseEvent<HTMLDivElement>) => {
    const target = e.target as HTMLElement;
    const copy = target.closest<HTMLElement>("[data-copy]");
    if (copy) {
      void copyText(codes[copy.dataset.copy!] ?? "").then(() => toast("Code copied"));
      return;
    }
    const a = target.closest<HTMLAnchorElement>("a[data-external]");
    if (a) {
      e.preventDefault();
      void fetch("/api/open", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ url: a.href }) });
    }
  };
  return <div className={`lp-md ${className}`} onClick={onClick} dangerouslySetInnerHTML={{ __html: html }} />;
});
