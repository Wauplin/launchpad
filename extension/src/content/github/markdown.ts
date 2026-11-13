// Agent Markdown (notes, answers) for the GitHub layer's widgets, built with `h()` from text only: headings, paragraphs,
// lists, quotes, fenced code, tables kept as text, and inline code, bold, italics and http(s) links. Never parses HTML,
// so a note can hold no markup of its own; anything unknown stays plain text.

import { h } from "./widgets";

const LINK = /^https?:\/\//i;

/** Inline spans: `code`, **bold**, *italic* or _italic_, [text](https://…). */
function inline(text: string): (Node | string)[] {
  const out: (Node | string)[] = [];
  const re = /(`+)([^`]+?)\1|\*\*([^*]+)\*\*|__([^_]+)__|\*([^*\s][^*]*)\*|(?<![\w])_([^_\s][^_]*)_(?![\w])|\[([^\]]+)\]\(([^)\s]+)\)/g;
  let at = 0;
  for (const m of text.matchAll(re)) {
    if (m.index > at) out.push(text.slice(at, m.index));
    if (m[2] !== undefined) out.push(h("code", { text: m[2] }));
    else if (m[3] !== undefined || m[4] !== undefined) out.push(h("strong", {}, ...inline(m[3] ?? m[4] ?? "")));
    else if (m[5] !== undefined || m[6] !== undefined) out.push(h("em", {}, ...inline(m[5] ?? m[6] ?? "")));
    else if (m[7] !== undefined && LINK.test(m[8] ?? "")) out.push(h("a", { attrs: { href: m[8]!, target: "_blank", rel: "noopener noreferrer" } }, ...inline(m[7])));
    else out.push(m[0]);
    at = m.index + m[0].length;
  }
  if (at < text.length) out.push(text.slice(at));
  return out;
}

/** The blocks of `md`, in a `div.lp-md`. */
export function markdown(md: string): HTMLElement {
  const root = h("div", { class: "lp-md" });
  const lines = md.replace(/\r\n?/g, "\n").split("\n");
  let i = 0;
  const para: string[] = [];
  const flush = () => { if (para.length) { root.append(h("p", {}, ...inline(para.join(" ")))); para.length = 0; } };
  while (i < lines.length) {
    const line = lines[i]!;
    const fence = line.match(/^\s*(```+|~~~+)/);
    if (fence) {
      flush();
      const body: string[] = [];
      i++;
      while (i < lines.length && !lines[i]!.trim().startsWith(fence[1]!)) body.push(lines[i++]!);
      i++;
      root.append(h("pre", {}, h("code", { text: body.join("\n") })));
      continue;
    }
    if (!line.trim()) { flush(); i++; continue; }
    const head = line.match(/^(#{1,6})\s+(.*)$/);
    if (head) { flush(); root.append(h(head[1]!.length <= 2 ? "h3" : "h4", {}, ...inline(head[2]!.replace(/\s#+$/, "")))); i++; continue; }
    if (/^\s*([-*_])(\s*\1){2,}\s*$/.test(line)) { flush(); root.append(h("hr")); i++; continue; }
    if (/^\s*>/.test(line)) {
      flush();
      const body: string[] = [];
      while (i < lines.length && /^\s*>/.test(lines[i]!)) body.push(lines[i++]!.replace(/^\s*>\s?/, ""));
      root.append(h("blockquote", {}, markdown(body.join("\n"))));
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
          const li = h("li", {}, ...(task ? [task[1] === " " ? "☐ " : "☑ ", ...inline(task[2]!)] : inline(m[3]!)));
          if (nested && current) {
            let sub = current.querySelector(":scope > ul, :scope > ol");
            if (!sub) { sub = h(/\d/.test(m[2]!) ? "ol" : "ul"); current.append(sub); }
            sub.append(li);
          } else { list.append(li); current = li; }
          i++;
        } else if (l.trim() && /^\s{2,}/.test(l) && current) {
          current.append(" ", ...inline(l.trim()));
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
