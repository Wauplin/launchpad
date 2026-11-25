// Pure: a small line and word diff for ```suggestion blocks (the original lines against the suggested ones), like
// GitHub's suggested-change view. A plain LCS: suggestions are a few lines, so sizes are capped instead of optimized.

/** A piece of a line; `changed` marks the words that differ from the paired line. */
export type Part = { text: string; changed: boolean };
export type DiffRow = { kind: "del" | "add" | "same"; parts: Part[] };

/** Above this many cells, the LCS table is skipped: everything counts as changed. */
const MAX_CELLS = 250_000;

/** The longest common subsequence of `a` and `b` as a list of steps (`same`, `del` from a, `add` from b), or null when too large. */
function lcs<T>(a: T[], b: T[]): ("same" | "del" | "add")[] | null {
  const n = a.length, m = b.length;
  if ((n + 1) * (m + 1) > MAX_CELLS) return null;
  const t: Uint32Array[] = Array.from({ length: n + 1 }, () => new Uint32Array(m + 1));
  for (let i = n - 1; i >= 0; i--) for (let j = m - 1; j >= 0; j--) t[i]![j] = a[i] === b[j] ? t[i + 1]![j + 1]! + 1 : Math.max(t[i + 1]![j]!, t[i]![j + 1]!);
  const out: ("same" | "del" | "add")[] = [];
  let i = 0, j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) { out.push("same"); i++; j++; }
    else if (t[i + 1]![j]! >= t[i]![j + 1]!) { out.push("del"); i++; }
    else { out.push("add"); j++; }
  }
  while (i < n) { out.push("del"); i++; }
  while (j < m) { out.push("add"); j++; }
  return out;
}

/** Words, runs of spaces and single punctuation marks. */
const tokens = (s: string) => s.match(/\s+|[\p{L}\p{N}_]+|[^\s\p{L}\p{N}_]/gu) ?? [];

/** Adjacent parts of the same kind merged. */
function merge(parts: Part[]): Part[] {
  const out: Part[] = [];
  for (const p of parts) {
    const last = out.at(-1);
    if (last && last.changed === p.changed) last.text += p.text;
    else if (p.text) out.push({ ...p });
  }
  return out;
}

/** The changed words of a removed line `a` and its added line `b`. When they share almost nothing, the whole lines are left plain. */
export function wordDiff(a: string, b: string): { del: Part[]; add: Part[] } {
  const x = tokens(a), y = tokens(b);
  const steps = lcs(x, y);
  if (!steps) return { del: [{ text: a, changed: false }], add: [{ text: b, changed: false }] };
  const del: Part[] = [], add: Part[] = [];
  let i = 0, j = 0, same = 0;
  for (const s of steps) {
    if (s === "same") { del.push({ text: x[i]!, changed: false }); add.push({ text: y[j]!, changed: false }); if (x[i]!.trim()) same += x[i]!.length; i++; j++; }
    else if (s === "del") del.push({ text: x[i++]!, changed: true });
    else add.push({ text: y[j++]!, changed: true });
  }
  // Two unrelated lines: marking every word says nothing more than the red and green rows.
  if (same < 0.25 * Math.min(a.replace(/\s/g, "").length, b.replace(/\s/g, "").length)) return { del: [{ text: a, changed: false }], add: [{ text: b, changed: false }] };
  return { del: merge(del), add: merge(add) };
}

/** The rows of the diff from `before` to `after`: kept lines, then each run of removed lines before its added ones,
 * the n-th removed and n-th added line of a run paired for the word diff. */
export function lineDiff(before: string[], after: string[]): DiffRow[] {
  const steps = lcs(before, after) ?? [...before.map(() => "del" as const), ...after.map(() => "add" as const)];
  const rows: DiffRow[] = [];
  let i = 0, j = 0, dels: string[] = [], adds: string[] = [];
  const flush = () => {
    const pairs = Math.min(dels.length, adds.length);
    const words = Array.from({ length: pairs }, (_, k) => wordDiff(dels[k]!, adds[k]!));
    dels.forEach((d, k) => rows.push({ kind: "del", parts: k < pairs ? words[k]!.del : [{ text: d, changed: false }] }));
    adds.forEach((a, k) => rows.push({ kind: "add", parts: k < pairs ? words[k]!.add : [{ text: a, changed: false }] }));
    dels = []; adds = [];
  };
  for (const s of steps) {
    if (s === "same") { flush(); rows.push({ kind: "same", parts: [{ text: before[i]!, changed: false }] }); i++; j++; }
    else if (s === "del") dels.push(before[i++]!);
    else adds.push(after[j++]!);
  }
  flush();
  return rows;
}
