// Agent Markdown (notes, answers) for the GitHub layer's widgets, built with `h()` from text only: headings, paragraphs,
// lists, quotes, fenced code, tables kept as text, and inline code, bold, italics and http(s) links. References show
// like in GitHub's comments (refs.ts): bare URLs, `#123`, `owner/repo#123`, `@login`, short GitHub URLs and links to
// private notes. Never parses HTML, so a note can hold no markup of its own; anything unknown stays plain text.

import { linkRef, tokens, type Here, type NoteRef, type Ref } from "./refs";
import { h } from "./widgets";

/** Where the text is shown: its item (for `#123` and short labels), and what a note link does. */
export type MdOptions = {
  here?: Here;
  /** The label of a note link (its author on this page), else the short one (`#12 (note)`). */
  noteLabel?: (note: NoteRef) => string | null;
  /** A plain click on a note link: true when it was handled on the page (no navigation). */
  onNote?: (note: NoteRef) => boolean;
  /** A ```suggestion block of a line comment (its lines): the suggested change as a diff (`suggestionDiff.ts`). */
  suggestion?: ((body: string[]) => HTMLElement) | null;
};

/** A link to a reference: GitHub's links open in this tab like GitHub's own, others in a new one. */
function anchor(ref: Ref, children: (Node | string)[], o: MdOptions): HTMLAnchorElement {
  const a = h("a", { attrs: { href: ref.href, ...(ref.github ? {} : { target: "_blank", rel: "noopener noreferrer" }) } }, ...children);
  const note = ref.note;
  if (note) {
    a.className = "lp-note-link";
    a.title = "Private note";
    a.addEventListener("click", (ev) => {
      if (ev.button !== 0 || ev.ctrlKey || ev.metaKey || ev.shiftKey || ev.altKey) return;
      if (o.onNote?.(note)) ev.preventDefault();
    });
  }
  return a;
}

/** Plain text with its references as links (`link` false: inside a link's own text). */
function linked(text: string, o: MdOptions, link: boolean): (Node | string)[] {
  if (!link) return [text];
  return tokens(text, o.here ?? null).map((t) => (typeof t === "string" ? t : anchor(t, [t.note ? o.noteLabel?.(t.note) ?? t.label : t.label], o)));
}

/** Inline spans: `code`, **bold**, *italic* or _italic_, [text](https://… or #lp-note-<id>), and references. */
function inline(text: string, o: MdOptions, link = true): (Node | string)[] {
  const out: (Node | string)[] = [];
  const re = /(`+)([^`]+?)\1|\*\*([^*]+)\*\*|__([^_]+)__|\*([^*\s][^*]*)\*|(?<![\w])_([^_\s][^_]*)_(?![\w])|\[([^\]]+)\]\(([^)\s]+)\)/g;
  let at = 0;
  for (const m of text.matchAll(re)) {
    const ref = m[7] !== undefined ? linkRef(m[8] ?? "", o.here ?? null) : null;
    // A URL's own `_` or `*` is not emphasis: the URL is linked whole.
    if ((m[3] ?? m[4] ?? m[5] ?? m[6]) !== undefined && /https?:\/\/\S*$/.test(text.slice(0, m.index))) continue;
    if (m.index > at) out.push(...linked(text.slice(at, m.index), o, link));
    if (m[2] !== undefined) out.push(h("code", { text: m[2] }));
    else if (m[3] !== undefined || m[4] !== undefined) out.push(h("strong", {}, ...inline(m[3] ?? m[4] ?? "", o, link)));
    else if (m[5] !== undefined || m[6] !== undefined) out.push(h("em", {}, ...inline(m[5] ?? m[6] ?? "", o, link)));
    else if (ref && link) out.push(anchor(ref, inline(m[7]!, o, false), o));
    else out.push(m[0]);
    at = m.index + m[0].length;
  }
  if (at < text.length) out.push(...linked(text.slice(at), o, link));
  return out;
}

/** The blocks of `md`, in a `div.lp-md`. */
export function markdown(md: string, o: MdOptions = {}): HTMLElement {
  const root = h("div", { class: "lp-md" });
  const lines = md.replace(/\r\n?/g, "\n").split("\n");
  let i = 0;
  const para: string[] = [];
  const flush = () => { if (para.length) { root.append(h("p", {}, ...inline(para.join(" "), o))); para.length = 0; } };
  while (i < lines.length) {
    const line = lines[i]!;
    const fence = line.match(/^\s*(```+|~~~+)/);
    if (fence) {
      flush();
      const body: string[] = [];
      i++;
      while (i < lines.length && !lines[i]!.trim().startsWith(fence[1]!)) body.push(lines[i++]!);
      i++;
      if (o.suggestion && line.trim().slice(fence[1]!.length).trim() === "suggestion") root.append(o.suggestion(body));
      else root.append(h("pre", {}, h("code", { text: body.join("\n") })));
      continue;
    }
    if (!line.trim()) { flush(); i++; continue; }
    const head = line.match(/^(#{1,6})\s+(.*)$/);
    if (head) { flush(); root.append(h(head[1]!.length <= 2 ? "h3" : "h4", {}, ...inline(head[2]!.replace(/\s#+$/, ""), o))); i++; continue; }
    if (/^\s*([-*_])(\s*\1){2,}\s*$/.test(line)) { flush(); root.append(h("hr")); i++; continue; }
    if (/^\s*>/.test(line)) {
      flush();
      const body: string[] = [];
      while (i < lines.length && /^\s*>/.test(lines[i]!)) body.push(lines[i++]!.replace(/^\s*>\s?/, ""));
      root.append(h("blockquote", {}, markdown(body.join("\n"), o)));
      continue;
    }
    const item = /^(\s*)([-*+]|\d+[.)])\s+(.*)$/;
    const first = line.match(item);
    if (first) {
      flush();
      const ordered = /\d/.test(first[2]!);
      const list = h(ordered ? "ol" : "ul");
      let current: HTMLElement | null = null;
      while (i < lines.length) {
        const l = lines[i]!;
        const m = l.match(item);
        if (m && (/\d/.test(m[2]!) === ordered || m[1]!.length > first[1]!.length)) {
          const nested = m[1]!.length > first[1]!.length + 1;
          const task = m[3]!.match(/^\[([ xX])\]\s+(.*)$/);
          const li = h("li", {}, ...(task ? [task[1] === " " ? "☐ " : "☑ ", ...inline(task[2]!, o)] : inline(m[3]!, o)));
          if (nested && current) {
            let sub = current.querySelector(":scope > ul, :scope > ol");
            if (!sub) { sub = h(/\d/.test(m[2]!) ? "ol" : "ul"); current.append(sub); }
            sub.append(li);
          } else { list.append(li); current = li; }
          i++;
        } else if (l.trim() && /^\s{2,}/.test(l) && current) {
          current.append(" ", ...inline(l.trim(), o));
          i++;
        } else break;
      }
      root.append(list);
      continue;
    }
    if (/^\s*\|.*\|\s*$/.test(line)) {
      flush();
      const rows: string[] = [];
      while (i < lines.length && /^\s*\|.*\|\s*$/.test(lines[i]!)) rows.push(lines[i++]!.trim());
      root.append(h("pre", { class: "lp-table" }, h("code", { text: rows.filter((r) => !/^\|[\s:|-]+\|$/.test(r)).join("\n") })));
      continue;
    }
    para.push(line.trim());
    i++;
  }
  flush();
  return root;
}
