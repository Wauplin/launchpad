// A line comment's ```suggestion block as a red and green diff, like GitHub's suggested change: the commented lines
// of the file at the suggestion's head (read live from github.com with the owner's session, kept per repository, head
// and path for the page's life), then the suggested lines, with the changed words marked. Shown only; the original
// lines are never sent anywhere. While the file loads, or when it cannot be read, the block stays a plain code block.

import { lineDiff } from "./lineDiff";
import { h } from "./widgets";

/** Where a line comment sits: its pull request, head and lines. */
export type LinePlace = { repo: string; number: number; head: string | null; path: string; line: number; start_line: number | null; side: string };

/** Files read at a head: their lines, or null when GitHub did not give them. */
const files = new Map<string, Promise<string[] | null>>();
const read = new Map<string, string[] | null>();
const MAX_BYTES = 2_000_000;

function fileLines(repo: string, ref: string, path: string): Promise<string[] | null> {
  const key = `${repo.toLowerCase()}@${ref}:${path}`;
  let p = files.get(key);
  if (!p) {
    const url = `https://github.com/${repo}/raw/${ref.split("/").map(encodeURIComponent).join("/")}/${path.split("/").map(encodeURIComponent).join("/")}`;
    // Same origin with the owner's GitHub session; GitHub's redirect to its raw host carries its own token.
    p = fetch(url, { credentials: "same-origin" })
      .then(async (r) => {
        if (!r.ok || Number(r.headers.get("content-length") ?? 0) > MAX_BYTES) return null;
        const text = await r.text();
        return text.length > MAX_BYTES || text.includes("\0") ? null : text.replace(/\r\n?/g, "\n").split("\n");
      })
      .catch(() => null)
      .then((lines) => { read.set(key, lines); return lines; });
    files.set(key, p);
  }
  return p;
}

/** The plain block shown before the original lines are known, or when they cannot be. */
const plain = (body: string[]) => h("pre", {}, h("code", { text: body.join("\n") }));

function diffView(before: string[], after: string[]): HTMLElement {
  const box = h("div", { class: "lp-diff", attrs: { role: "table", "aria-label": "Suggested change" } });
  box.append(h("div", { class: "lp-diff-head", text: "Suggested change" }));
  for (const row of lineDiff(before, after)) {
    const sign = row.kind === "del" ? "-" : row.kind === "add" ? "+" : " ";
    const text = h("span", { class: "lp-diff-text" }, ...row.parts.map((p) => (p.changed ? h("span", { class: "lp-diff-word", text: p.text }) : p.text)));
    if (!row.parts.some((p) => p.text)) text.append("\n");
    box.append(h("div", { class: `lp-diff-row lp-diff-${row.kind}`, attrs: { role: "row" } }, h("span", { class: "lp-diff-sign", text: sign, attrs: { "aria-hidden": "true" } }), text));
  }
  return box;
}

/** The renderer of a line comment's ```suggestion blocks: a diff when the commented lines can be read, else the plain
 * block (replaced in place once the file arrives). Null for a comment on removed lines, which GitHub cannot apply. */
export function suggestionRenderer(place: LinePlace): ((body: string[]) => HTMLElement) | null {
  if (place.side === "LEFT") return null;
  const ref = place.head ?? `refs/pull/${place.number}/head`;
  const key = `${place.repo.toLowerCase()}@${ref}:${place.path}`;
  const start = place.start_line ?? place.line;
  const cut = (lines: string[] | null) => (lines && start >= 1 && place.line >= start && place.line <= lines.length ? lines.slice(start - 1, place.line) : null);
  return (body) => {
    if (read.has(key)) {
      const before = cut(read.get(key)!);
      return before ? diffView(before, body) : plain(body);
    }
    const slot = h("div", { class: "lp-diff-slot" }, plain(body));
    void fileLines(place.repo, ref, place.path).then((lines) => {
      const before = cut(lines);
      if (before) slot.replaceChildren(diffView(before, body));
    });
    return slot;
  };
}
