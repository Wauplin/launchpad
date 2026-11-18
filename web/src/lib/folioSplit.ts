// The line-based block splitter of folio pages: the UI's twin of crates/automation/src/folio_split.rs. Both produce
// the same outline for the fixture pages of web/tests/folio-split/ (`pnpm test:folios` and `cargo test`), so the
// reader and Home's check agree on where every heading, container and fence starts. Keep the rules equal:
// - Front matter: line 1 is `---`, closed by the next `---` or `...` line.
// - Fences: ``` or ~~~ (3 or more, up to 3 spaces of indent), closed by the same character at least as long.
// - Containers: `:::name{attrs} title` opens; a line of only colons closes the innermost open container when it has
//   at least as many colons.
// - Headings: ATX `#`…`######` outside fences, with an optional `{#id}`; repeats get `-1`, `-2`.
// No imports: tests load this file directly with Node.

export type OutlineItem =
  | { t: "heading"; line: number; level: number; id: string; text: string; explicit: boolean }
  | { t: "container"; line: number; end: number; name: string; attrs: string; title: string; depth: number; closed: boolean }
  | { t: "fence"; line: number; end: number; lang: string; info: string; depth: number; closed: boolean };

export type Outline = { front: { start: number; end: number } | null; items: OutlineItem[]; problems: { line: number; code: string }[] };

/** The lines of a text, without their line endings (a final newline adds no empty line). */
export function lines(text: string): string[] {
  const v = text.split("\n").map((l) => (l.endsWith("\r") ? l.slice(0, -1) : l));
  if (text.endsWith("\n")) v.pop();
  return v;
}

const lead = (s: string, c: string) => { let n = 0; while (n < s.length && s[n] === c) n++; return n; };
const indentOk = (line: string): string | null => { const n = lead(line, " "); return n <= 3 ? line.slice(n) : null; };

export function fenceOpen(line: string): { c: string; len: number; info: string } | null {
  const rest = indentOk(line);
  if (rest == null) return null;
  const c = rest[0];
  if (c !== "`" && c !== "~") return null;
  const n = lead(rest, c);
  if (n < 3) return null;
  const info = rest.slice(n).trim();
  if (c === "`" && info.includes("`")) return null;
  return { c, len: n, info };
}

function fenceClose(line: string, c: string, len: number): boolean {
  const rest = indentOk(line);
  if (rest == null) return false;
  const n = lead(rest, c);
  return n >= len && rest.slice(n).trim() === "";
}

export function containerOpen(line: string): { colons: number; name: string; attrs: string; title: string } | null {
  const n = lead(line, ":");
  if (n < 3) return null;
  const rest = line.slice(n).trimStart();
  const m = /^[A-Za-z0-9_-]*/.exec(rest)!;
  const name = m[0];
  if (!name || !/^[A-Za-z]/.test(name)) return null;
  let tail = rest.slice(name.length).trimStart();
  let attrs = "";
  if (tail.startsWith("{")) {
    const end = tail.indexOf("}");
    if (end >= 0) { attrs = tail.slice(0, end + 1); tail = tail.slice(end + 1).trimStart(); }
  }
  return { colons: n, name, attrs, title: tail.trimEnd() };
}

function containerClose(line: string): number | null {
  const n = lead(line, ":");
  return n >= 3 && line.slice(n).trim() === "" ? n : null;
}

export const idChar = (c: string) => /^[A-Za-z0-9_.:-]$/.test(c);

export function heading(line: string): { level: number; text: string; id: string | null } | null {
  const rest = indentOk(line);
  if (rest == null) return null;
  const n = lead(rest, "#");
  if (n === 0 || n > 6) return null;
  const after = rest.slice(n);
  if (after && after[0] !== " " && after[0] !== "\t") return null;
  let text = after.trim();
  const stripped = text.replace(/#+$/, "");
  if (stripped.length < text.length && (stripped === "" || /[ \t]$/.test(stripped))) text = stripped.trimEnd();
  let id: string | null = null;
  if (text.endsWith("}")) {
    const open = text.lastIndexOf("{#");
    if (open >= 0) {
      const cand = text.slice(open + 2, -1);
      if (cand && [...cand].every(idChar)) { id = cand; text = text.slice(0, open).trimEnd(); }
    }
  }
  return { level: n, text, id };
}

/** The anchor id of a heading's text (see folio_split.rs `slug`). */
export function slug(text: string): string {
  let plain = "";
  const chars = [...text];
  for (let i = 0; i < chars.length; i++) {
    if (chars[i] === "]" && chars[i + 1] === "(") {
      i++;
      while (i < chars.length && chars[i] !== ")") i++;
      continue;
    }
    plain += chars[i];
  }
  let out = "";
  for (const c of plain.toLowerCase()) {
    if (/[\p{L}\p{N}_]/u.test(c)) out += c;
    else if (c === " " || c === "-" || c === "\t") out += "-";
  }
  return out.replace(/^-+|-+$/g, "") === "" ? "section" : out;
}

/** Splits a page into its outline. */
export function outline(text: string): Outline {
  const ls = lines(text);
  const items: OutlineItem[] = [];
  const problems: { line: number; code: string }[] = [];
  let front: Outline["front"] = null;
  let i = 0;
  if (ls.length && ls[0].trimEnd() === "---") {
    const p = ls.slice(1).findIndex((l) => l.trimEnd() === "---" || l.trimEnd() === "...");
    if (p >= 0) { front = { start: 1, end: p + 2 }; i = p + 2; }
    else problems.push({ line: 1, code: "F101" });
  }
  const stack: [number, number][] = [];
  const seen = new Map<string, number>();
  while (i < ls.length) {
    const line = ls[i];
    const no = i + 1;
    const f = fenceOpen(line);
    if (f) {
      const lang = f.info.split(/\s+/)[0] ?? "";
      let j = i + 1;
      while (j < ls.length && !fenceClose(ls[j], f.c, f.len)) j++;
      const closed = j < ls.length;
      if (!closed) problems.push({ line: no, code: "F102" });
      items.push({ t: "fence", line: no, end: closed ? j + 1 : ls.length, lang, info: f.info, depth: stack.length, closed });
      i = j + 1;
      continue;
    }
    const close = containerClose(line);
    if (close != null) {
      const top = stack[stack.length - 1];
      if (top && top[1] <= close) {
        stack.pop();
        const c = items[top[0]];
        if (c.t === "container") { c.end = no; c.closed = true; }
      }
      i++;
      continue;
    }
    const open = containerOpen(line);
    if (open) {
      stack.push([items.length, open.colons]);
      items.push({ t: "container", line: no, end: ls.length, name: open.name, attrs: open.attrs, title: open.title, depth: stack.length - 1, closed: false });
      i++;
      continue;
    }
    const h = heading(line);
    if (h) {
      const base = h.id ?? slug(h.text);
      const n = seen.get(base) ?? 0;
      seen.set(base, n + 1);
      items.push({ t: "heading", line: no, level: h.level, id: n === 0 ? base : `${base}-${n}`, text: h.text, explicit: h.id != null });
    }
    i++;
  }
  for (const [idx] of stack) problems.push({ line: items[idx].line, code: "F102" });
  problems.sort((a, b) => a.line - b.line);
  return { front, items, problems };
}

export type Attrs = { id: string | null; classes: string[]; pairs: Record<string, string>; flags: string[] };

/** `{#id .class key=value key="two words" flag}`. */
export function attrs(raw: string): Attrs {
  const out: Attrs = { id: null, classes: [], pairs: {}, flags: [] };
  let rest = raw.trim().replace(/^\{/, "").replace(/\}$/, "").trim();
  while (rest) {
    let word = "";
    const eq = rest.indexOf("=");
    const ws = rest.search(/\s/);
    if (eq > 0 && (ws < 0 || eq < ws)) {
      const key = rest.slice(0, eq);
      let after = rest.slice(eq + 1);
      let value: string;
      if (after.startsWith("\"")) {
        const e = after.indexOf("\"", 1);
        value = e >= 0 ? after.slice(1, e) : after.slice(1);
        after = e >= 0 ? after.slice(e + 1) : "";
      } else {
        const e = after.search(/\s/);
        value = e >= 0 ? after.slice(0, e) : after;
        after = e >= 0 ? after.slice(e) : "";
      }
      out.pairs[key] = value;
      rest = after.trimStart();
      continue;
    }
    word = ws >= 0 ? rest.slice(0, ws) : rest;
    rest = ws >= 0 ? rest.slice(ws).trimStart() : "";
    if (word.startsWith("#")) out.id = word.slice(1);
    else if (word.startsWith(".")) out.classes.push(word.slice(1));
    else if (word) out.flags.push(word);
  }
  return out;
}
