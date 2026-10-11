// Display-only folding on top of rows.ts: consecutive notices become one line, and the launch
// prompt shows the typed text with the appended Launchpad blocks folded.

import type { Row } from "./rows";

export type NoticesRow = { key: string; v: string; t: "notices"; texts: string[] };
export type TraceRow = (Row & { instructions?: string[] }) | NoticesRow;

/**
 * The launch prompt starts with the typed text (compose.rs renders it first, then blank-line
 * separated context, presets and Launchpad instructions). Returns null when the text does not
 * start with the typed text or nothing was appended.
 */
export function splitPrompt(text: string, typed: string): { typed: string; instructions: string[] } | null {
  const t = typed.trim();
  if (t && !text.startsWith(t)) return null;
  const instructions = text.slice(t.length).split(/\n{2,}/).map((p) => p.trim()).filter(Boolean);
  return instructions.length ? { typed: t, instructions } : null;
}

export function foldTrace(rows: Row[], typed: string): TraceRow[] {
  const out: TraceRow[] = [];
  let first = true;
  for (const r of rows) {
    if (r.t === "user" && first) {
      first = false;
      const split = splitPrompt(r.text, typed);
      out.push(split ? { ...r, text: split.typed, instructions: split.instructions } : r);
      continue;
    }
    const last = out[out.length - 1];
    if (r.t === "notice" && last?.t === "notices") { last.texts.push(r.text); last.v = String(last.texts.length); continue; }
    if (r.t === "notice" && last?.t === "notice") { out[out.length - 1] = { key: last.key, v: "2", t: "notices", texts: [last.text, r.text] }; continue; }
    out.push(r);
  }
  return out;
}

/** "4 cost limit changes · now $0.02 (limit reached)" or "3 notices · latest: …". */
export function noticesSummary(texts: string[]): string {
  const last = texts[texts.length - 1];
  if (texts.every((t) => /^Cost limit (set|removed)/.test(t))) {
    const amount = /^Cost limit set to (\$[\d.]+)/.exec(last)?.[1];
    return `${texts.length} cost limit changes · now ${amount ? `${amount}${/limit is reached/.test(last) ? " (limit reached)" : ""}` : "no limit"}`;
  }
  if (texts.every((t) => /^(Model set to|Restarted with)/.test(t))) return `${texts.length} model changes · ${last}`;
  return `${texts.length} notices · latest: ${last}`;
}
