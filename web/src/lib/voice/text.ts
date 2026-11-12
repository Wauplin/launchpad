// Where dictated text lands: at the caret, with a space or a capital depending on the text around it, as one undo step.

/** The phrase as it goes between `before` and `after`. */
export function fitPhrase(before: string, phrase: string, after: string): string {
  let t = phrase.trim();
  if (!t) return "";
  const sentenceStart = !before.trim() || /[.!?]["')\]]?\s*$/.test(before) || /\n\s*$/.test(before);
  if (sentenceStart) t = t[0].toUpperCase() + t.slice(1);
  if (before && !/\s$/.test(before) && !/^[,.;:!?)\]]/.test(t)) t = ` ${t}`;
  if (after && !/^[\s,.;:!?)\]]/.test(after)) t = `${t} `;
  return t;
}

/** `text` with a phrase added at its end, spaced like an insertion at the caret. */
export function appendPhrase(text: string, phrase: string): string {
  return text + fitPhrase(text, phrase, "");
}

/** Inserts a settled phrase at the caret of `el`. A focused field gets a native edit (its own undo step, and
 * PromptField's history step); a blurred one (the phone keeps the keyboard closed) gets the value through the input
 * event React listens to. Returns the inserted text. */
export function insertAtCaret(el: HTMLTextAreaElement | HTMLInputElement, phrase: string): string {
  const start = el.selectionStart ?? el.value.length, end = el.selectionEnd ?? start;
  const text = fitPhrase(el.value.slice(0, start), phrase, el.value.slice(end));
  if (!text) return "";
  if (document.activeElement === el && !el.disabled) {
    try { if (document.execCommand("insertText", false, text)) return text; } catch { /* Fall back below. */ }
  }
  const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(proto, "value")!.set!.call(el, el.value.slice(0, start) + text + el.value.slice(end));
  el.setSelectionRange(start + text.length, start + text.length);
  el.dispatchEvent(new Event("input", { bubbles: true }));
  return text;
}

const words = (s: string) => s.toLowerCase().replace(/[-_.]+/g, " ").replace(/[^\p{L}\p{N} ]+/gu, "").trim().split(/\s+/).filter(Boolean);
const escape = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

/** "at Hubert JS" → "@hubert-js" when a member's name or slug matches the spoken words exactly. */
export function spokenMentions(text: string, members: { name: string; slug: string }[]): string {
  let out = text;
  // Longer names first, so "Hubert JS" wins over "Hubert".
  const named = members.flatMap((m) => [m.name, m.slug].map((n) => ({ slug: m.slug, w: words(n) }))).filter((m) => m.slug && m.w.length).sort((a, b) => b.w.length - a.w.length);
  for (const m of named) {
    const pattern = new RegExp(`(^|[\\s(])(?:at|@)\\s+${m.w.map(escape).join("[\\s\\-_.]*")}(?=$|[\\s,.;:!?)])`, "giu");
    out = out.replace(pattern, (_, lead: string) => `${lead}@${m.slug}`);
  }
  return out;
}
