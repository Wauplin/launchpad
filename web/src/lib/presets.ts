// Presets appended to a message sent to an existing session (steering, resuming, scheduled
// follow-ups). Same layout as the launch prompt in compose.rs: the typed text, a blank line, then
// the chosen preset texts in settings order joined by a space.

import type { Preset } from "@/gen/Preset";

export type PresetSurface = "agent" | "ask_launchpad";

/** Older config files keep their presets available to agents only. */
export function availablePresets(presets: Preset[], surface: PresetSurface): Preset[] {
  return presets.filter((p) => surface === "agent" ? p.agent !== false : p.ask_launchpad === true);
}

/** The chosen presets in settings order, skipping deleted ones and empty texts. */
export function chosenPresets(ids: string[], presets: Preset[]): Preset[] {
  return presets.filter((p) => ids.includes(p.id) && p.text.trim());
}

export function withPresets(text: string, ids: string[], presets: Preset[]): string {
  const block = chosenPresets(ids, presets).map((p) => p.text.trim()).join(" ");
  return [text.trim(), block].filter(Boolean).join("\n\n");
}

/**
 * Display-only inverse of `withPresets`: when the last paragraph of a message is exactly a
 * combination of preset texts in settings order, returns the text before it and those presets.
 */
export function splitPresets(text: string, presets: Preset[]): { text: string; presets: Preset[] } | null {
  const usable = presets.filter((p) => p.text.trim());
  if (!usable.length) return null;
  const at = text.lastIndexOf("\n\n");
  const head = at < 0 ? "" : text.slice(0, at).trim();
  let rest = (at < 0 ? text : text.slice(at + 2)).trim();
  const found: Preset[] = [];
  for (const p of usable) {
    const t = p.text.trim();
    if (rest === t || rest.startsWith(`${t} `)) {
      found.push(p);
      rest = rest.slice(t.length).trimStart();
    }
  }
  return found.length && !rest ? { text: head, presets: found } : null;
}
