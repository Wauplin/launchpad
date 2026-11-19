import type { ItemAction } from "@/gen/ItemAction";
import type { SuggestionEdit } from "../../shared/messages";

const inFlight = new Set<string>();

/** Keep publication limited to untouched drafts; Home owns the publication lock and reviewed-head checks. */
export async function submitSuggestion(
  suggestion: { id: string; kind: string; status: string },
  actions: ItemAction[],
  send: (args: { id: string; actions: ItemAction[] }) => Promise<unknown>,
): Promise<unknown> {
  if (suggestion.status !== "draft") throw new Error("Check GitHub before attempting publication again.");
  if (actions.some((a) => a.kind === "close") && suggestion.kind === "review") throw new Error("Reviews cannot close a pull request without merging.");
  if (suggestion.kind === "action" && !actions.length) throw new Error("Every action is unticked: discard it instead.");
  if (inFlight.has(suggestion.id)) throw new Error("Publication is already in progress.");
  inFlight.add(suggestion.id);
  try {
    return await send({ id: suggestion.id, actions });
  } finally {
    inFlight.delete(suggestion.id);
  }
}

/** The owner's edit in progress: the comment's or review summary's text, and each line comment's text by its key
 * (`lineKeys`; null: left out of the review). */
export type EditDraft = { body: string; lines: Record<string, string | null> };

type Line = { part: string; path: string; line: number; side: string; start_line: number | null; body: string };
type Editable = { id: string; kind: string; body: string; event: string; merge: string | null; run_ci: boolean; lines: Line[] };

/** Pure: each line comment's key, its ledger part (`line:<path>:<line>:<side>`), with `#2`, `#3`… for more comments on
 * the same place, so a draft follows its comment even when another one leaves the list. */
export function lineKeys(lines: { part: string }[]): string[] {
  const seen = new Map<string, number>();
  return lines.map((l) => {
    const n = (seen.get(l.part) ?? 0) + 1;
    seen.set(l.part, n);
    return n === 1 ? l.part : `${l.part}#${n}`;
  });
}

/** Pure: a fresh edit of a suggestion, its raw text as the agent wrote it (Markdown and ```suggestion blocks as is). */
export function startEdit(s: Editable): EditDraft {
  const keys = lineKeys(s.lines);
  return { body: s.body, lines: Object.fromEntries(s.lines.map((l, i) => [keys[i]!, l.body])) };
}

/** Pure: a line comment's text in a draft: typed, the agent's (not in the draft), or null (left out). */
export function lineText(d: EditDraft, key: string, l: { body: string }): string | null {
  return Object.hasOwn(d.lines, key) ? d.lines[key]! : l.body;
}

/** Pure: what Save sends (`suggestion.edit`): the text typed, a review's event and follow-up unchanged, and its line
 * comments at their places with their new text (left-out ones removed); a comment sends no line comments. */
export function editRequest(s: Editable, d: EditDraft) {
  const base = { id: s.id, body: d.body, event: s.event as SuggestionEdit["event"], merge: s.merge as SuggestionEdit["merge"], run_ci: s.run_ci };
  if (s.kind !== "review") return base;
  const keys = lineKeys(s.lines);
  const comments = s.lines.flatMap((l, i) => {
    const body = lineText(d, keys[i]!, l);
    return body === null ? [] : [{ path: l.path, line: l.line, side: l.side as "RIGHT" | "LEFT", start_line: l.start_line, body }];
  });
  return { ...base, comments };
}

/** Pure: whether a draft differs from the suggestion (Save is pointless otherwise). */
export function edited(s: Editable, d: EditDraft): boolean {
  const keys = lineKeys(s.lines);
  return d.body !== s.body || s.lines.some((l, i) => lineText(d, keys[i]!, l) !== l.body);
}
