// Who wrote a message, at a glance: each bot keeps one color everywhere (chats, channels, lists), picked from its
// name so every window agrees without storing anything. The name and the initial always show too: color is never
// the only sign. Pure, so it is tested (tests/chatRows.test.mjs).

/** How many identity colors the theme has (`--color-id-<n>` and `--color-id-<n>-soft` in styles.css). */
export const IDENTITY_TONES = 8;

/** The color of a name: the same for the same name (case and outer spaces aside), spread over the palette. */
export function identityTone(name: string): number {
  let h = 0x811c9dc5;
  for (const c of name.trim().toLowerCase()) {
    h ^= c.codePointAt(0) ?? 0;
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h % IDENTITY_TONES;
}

/** The first letter of a name, for its mark. */
export const initialOf = (name: string, fallback = "B") => ([...name.trim()][0] ?? fallback).toUpperCase();
