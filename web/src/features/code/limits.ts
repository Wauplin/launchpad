// Bound both tokenization and browser layout. A small minified file can still be expensive.
export const MAX_HIGHLIGHT_CHARS = 100 * 1024;
export const MAX_HIGHLIGHT_LINES = 10_000;
export const MAX_HIGHLIGHT_LINE = 2_000;
const MAX_DISPLAY_LINES = 100_000;
const MAX_DISPLAY_LINE = 20_000;

export function textLimits(text: string): { display: string | null; colors: boolean } {
  let lines = 1;
  let start = 0;
  let colors = text.length <= MAX_HIGHLIGHT_CHARS;
  for (let i = 0; i <= text.length; i++) {
    if (i !== text.length && text.charCodeAt(i) !== 10) continue;
    const length = i - start;
    if (length > MAX_DISPLAY_LINE) return { display: "This file has a line longer than 20,000 characters: too large to show here.", colors: false };
    if (length > MAX_HIGHLIGHT_LINE) colors = false;
    if (i !== text.length) lines++;
    if (lines > MAX_DISPLAY_LINES) return { display: "This file has more than 100,000 lines: too large to show here.", colors: false };
    start = i + 1;
  }
  return { display: null, colors: colors && lines <= MAX_HIGHLIGHT_LINES };
}
