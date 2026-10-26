import type { MarkdownIt } from "markdown-it";

export type FileCitation = { path: string; line: number | null; href: string; downloadHref: string };
export type FileCitations = { citation?: (path: string) => FileCitation | null };

/** Explicit citations may name ignored outputs, but must stay under the session folder.
 * The daemon independently checks the registered root and the resolved path, including symlinks.
 */
export function citationPath(raw: string, cwd: string): string | null {
  const location = cwd.replaceAll("\\", "/").replace(/\/$/, "");
  const remote = /^([a-z0-9][a-z0-9._-]{1,62}):(\/.*)$/.exec(location);
  const root = remote?.[2] ?? location;
  let path = raw.replaceAll("\\", "/");
  if (remote && path.startsWith(`${remote[1]}:`)) path = path.slice(remote[1].length + 1);
  if (path.startsWith(`${root}/`)) path = path.slice(root.length + 1);
  else if (path.startsWith("/") || /^[A-Za-z]:/.test(path) || path.includes(":")) return null;
  path = path.replace(/^\.\//, "");
  if (!path || path.split("/").some((part) => !part || part === "." || part === "..") || /[\x00-\x1f]/.test(path)) return null;
  return path;
}

/** Parse native Codex directives as inline tokens, leaving examples in code and links untouched. */
export function installFileCitations(md: MarkdownIt) {
  md.inline.ruler.before("text", "codex_file_citation", (state, silent) => {
    if (state.linkLevel || !state.src.startsWith(":codex-file-citation{", state.pos)) return false;
    const match = /^:codex-file-citation\{((?:\s*[\w-]+="[^"\n]*")+\s*)\}/.exec(state.src.slice(state.pos));
    if (!match) return false;
    const attrs = new Map([...match[1].matchAll(/([\w-]+)="([^"\n]*)"/g)].map((m) => [m[1], m[2]]));
    const path = attrs.get("path");
    if (!path) return false;
    if (!silent) {
      const token = state.push("codex_file_citation", "", 0);
      token.content = path;
      token.meta = { output: attrs.get("purpose") === "output" };
    }
    state.pos += match[0].length;
    return true;
  });
  md.renderer.rules.codex_file_citation = (tokens, idx, _options, env) => {
    const token = tokens[idx];
    const escape = md.utils.escapeHtml;
    const hit = (env as { codeLinks?: FileCitations } | undefined)?.codeLinks?.citation?.(token.content);
    const label = escape(token.content.replaceAll("\\", "/").split("/").pop() || token.content);
    if (!hit) return `<code title="${escape(token.content)}">${escape(token.content)}</code>`;
    const view = `<a class="lp-path" data-path="${escape(hit.path)}" href="${escape(hit.href)}" title="${escape(token.content)}">${label}</a>`;
    const download = token.meta?.output ? ` <a href="${escape(hit.downloadHref)}" download title="Download file (up to 20 MiB)">Download</a>` : "";
    return `${view}${download}`;
  };
}
