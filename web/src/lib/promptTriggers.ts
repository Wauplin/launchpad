export type Mention = { trigger: "@" | "/" | "#"; start: number; query: string };
export function mentionAt(text: string, caret: number): Mention | null {
  let start = caret;
  while (start > 0 && !/\s/.test(text[start - 1])) start--;
  const word = text.slice(start, caret), trigger = word[0];
  if (trigger !== "@" && trigger !== "/" && trigger !== "#") return null;
  const query = word.slice(1);
  if (trigger === "#" && query.startsWith("#")) return null;
  if (trigger === "/" && (query.includes("/") || query.includes("\\"))) return null;
  return { trigger, start, query };
}
