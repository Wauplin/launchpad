// Pure: GitHub-style references in agent text (notes, answers), like GitHub's own comments: bare URLs become links,
// `#123`, `owner/repo#123` and `@login` link to github.com, and GitHub URLs show short (`#123`, `owner/repo#123 (comment)`,
// `abc1234`). A private note's link (`https://github.com/<repo>/issues/<n>#lp-note-<id>`, `notes::link` on Home) shows
// as `#123 (note)`, or by its author on its own item; the timeline scrolls to it instead of loading the page again.

/** The item the text is shown on (null: none, so `#123` stays text). */
export type Here = { repo: string; number: number } | null;

/** A private note named by a link. */
export type NoteRef = { repo: string; number: number; id: string };

/** A link found in text. */
export type Ref = { href: string; label: string; note: NoteRef | null; github: boolean };

const NAME = "[A-Za-z0-9-]+/[A-Za-z0-9._-]+";
const NOTE_FRAGMENT = /^#lp-note-([A-Za-z0-9-]{1,64})$/;

const sameRepo = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

/** Pure: the short label of a GitHub URL, like GitHub's autolinks (null: not one it shortens). */
export function githubRef(url: string, here: Here): Ref | null {
  let u: URL;
  try { u = new URL(url); } catch { return null; }
  if (u.protocol !== "https:" || (u.hostname !== "github.com" && u.hostname !== "www.github.com")) return null;
  const path = u.pathname.replace(/\/+$/, "");
  const item = path.match(new RegExp(`^/(${NAME})/(?:issues|pull)/(\\d+)$`));
  const commit = path.match(new RegExp(`^/(${NAME})/commit/([0-9a-f]{7,40})$`, "i"));
  const ref = (label: string, note: NoteRef | null = null): Ref => ({ href: url, label, note, github: true });
  if (item) {
    const [repo, n] = [item[1]!, Number(item[2])];
    const name = here && sameRepo(here.repo, repo) ? `#${n}` : `${repo}#${n}`;
    const note = u.hash.match(NOTE_FRAGMENT);
    if (note) return ref(`${name} (note)`, { repo, number: n, id: note[1]! });
    if (/^#(issuecomment-|discussion_r|r)\d+$/.test(u.hash)) return ref(`${name} (comment)`);
    if (/^#pullrequestreview-\d+$/.test(u.hash)) return ref(`${name} (review)`);
    return u.hash ? null : ref(name);
  }
  if (commit && !u.hash) {
    const short = commit[2]!.slice(0, 7);
    return ref(here && sameRepo(here.repo, commit[1]!) ? short : `${commit[1]}@${short}`);
  }
  return null;
}

/** Pure: a link's target (a Markdown link's or a bare URL): http(s) URLs, and `#lp-note-<id>` for a note of `here`. */
export function linkRef(href: string, here: Here): Ref | null {
  const note = href.match(NOTE_FRAGMENT);
  if (note) return here ? { href: `https://github.com/${here.repo}/issues/${here.number}${href}`, label: "note", note: { ...here, id: note[1]! }, github: true } : null;
  if (!/^https?:\/\//i.test(href)) return null;
  return githubRef(href, here) ?? { href, label: href, note: null, github: false };
}

/** Pure: a bare URL's end, like GitHub: without trailing punctuation or a `)` it does not open. */
function trimUrl(url: string): string {
  let u = url.replace(/[.,;:!?'"*_]+$/, "");
  while (u.endsWith(")") && (u.match(/\(/g)?.length ?? 0) < (u.match(/\)/g)?.length ?? 0)) u = u.slice(0, -1).replace(/[.,;:!?'"*_]+$/, "");
  return u;
}

/** Pure: the references of plain text (no Markdown): bare URLs, `owner/repo#123`, `#123` (with `here`) and `@login`. */
export function tokens(text: string, here: Here): (string | Ref)[] {
  const out: (string | Ref)[] = [];
  const re = new RegExp(`(https?://[^\\s<>]+)|(?<![\\w/.#@-])(${NAME})?#(\\d+)\\b|(?<![\\w@/.\`-])@([A-Za-z0-9](?:[A-Za-z0-9-]{0,38}))\\b(?![/@])`, "g");
  let at = 0;
  for (const m of text.matchAll(re)) {
    let ref: Ref | null = null;
    let raw = m[0];
    if (m[1] !== undefined) {
      raw = trimUrl(m[1]);
      ref = raw.length > 8 ? linkRef(raw, here) : null;
    } else if (m[3] !== undefined) {
      const repo = m[2] ?? here?.repo;
      if (repo) ref = { href: `https://github.com/${repo}/issues/${m[3]}`, label: raw, note: null, github: true };
    } else if (m[4] !== undefined) {
      ref = { href: `https://github.com/${m[4]}`, label: raw, note: null, github: true };
    }
    if (!ref) continue;
    if (m.index > at) out.push(text.slice(at, m.index));
    out.push(ref);
    at = m.index + raw.length;
  }
  if (at < text.length) out.push(text.slice(at));
  return out;
}

/** Pure: the IDs of the notes `text` links to on item `of` (its own references, for "Referenced in"). */
export function noteIdsIn(text: string, of: { repo: string; number: number }): string[] {
  const ids = new Set<string>();
  for (const m of text.matchAll(new RegExp(`(?:https://(?:www\\.)?github\\.com/(${NAME})/(?:issues|pull)/(\\d+))?#lp-note-([A-Za-z0-9-]{1,64})`, "g"))) {
    if (m[1] === undefined || (sameRepo(m[1], of.repo) && Number(m[2]) === of.number)) ids.add(m[3]!);
  }
  return [...ids];
}
