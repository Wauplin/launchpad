import type { MarkdownIt, Token } from "markdown-it";

// Word boundaries exclude hashes embedded in identifiers, file names and longer hex strings.
const OID = /(?<![\w/.-])[a-fA-F0-9]{7,40}(?!\w|[./-]\w)/g;
export type CommitLinks = Record<string, { sha: string; href: string }>;
type Env = { commits?: CommitLinks };

function inLink(tokens: Token[], idx: number) {
  let depth = 0;
  for (let i = 0; i < idx; i++) {
    if (tokens[i].type === "link_open") depth++;
    if (tokens[i].type === "link_close") depth--;
  }
  return depth > 0;
}

/** Only visible prose and inline code, never link destinations, images or fenced/indented code. */
export function commitCandidates(tokens: Token[]): string[] {
  const found = new Set<string>();
  for (const block of tokens) {
    const children = block.children ?? [];
    let depth = 0;
    for (const token of children) {
      if (token.type === "link_open") depth++;
      if (token.type === "link_close") depth--;
      if ((token.type === "text" || token.type === "code_inline") && depth === 0) {
        for (const match of token.content.matchAll(OID)) found.add(match[0]);
      }
    }
  }
  return [...found].sort();
}

/** Wrap the existing renderers so inline file links continue to work. */
export function installCommitLinks(md: MarkdownIt) {
  for (const kind of ["text", "code_inline"]) {
    const original = md.renderer.rules[kind]!;
    md.renderer.rules[kind] = (tokens, idx, options, env, self) => {
      const content = tokens[idx].content;
      const commits = (env as Env | undefined)?.commits;
      if (!commits) return original(tokens, idx, options, env, self);
      const matches = [...content.matchAll(OID)];
      if (!matches.some((match) => commits[match[0]]) || inLink(tokens, idx)) return original(tokens, idx, options, env, self);
      let cursor = 0;
      let html = "";
      let linked = false;
      for (const match of matches) {
        const hit = commits[match[0]];
        if (!hit) continue;
        linked = true;
        html += md.utils.escapeHtml(content.slice(cursor, match.index));
        html += `<a class="lp-path" data-commit="${md.utils.escapeHtml(hit.sha)}" href="${md.utils.escapeHtml(hit.href)}" title="Open commit in the code viewer">${match[0]}</a>`;
        cursor = match.index + match[0].length;
      }
      if (!linked) return original(tokens, idx, options, env, self);
      html += md.utils.escapeHtml(content.slice(cursor));
      return kind === "code_inline" ? `<code>${html}</code>` : html;
    };
  }
}
