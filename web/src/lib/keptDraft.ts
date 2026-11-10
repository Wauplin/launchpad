// Text typed into an editor survives back: the draft is kept in this window's sessionStorage, keyed by the record
// and the version it started from (`launchpad.draft.<key>`), until it is saved or discarded. Reopening the editor
// on the same version brings it back with a "Draft kept" line; a newer version starts from the new text.

import { useEffect, useRef, useState } from "react";
import { storageAllowed } from "./clientStorage";

const PREFIX = "launchpad.draft.";

export function readDraft(key: string): string | null {
  try { return sessionStorage.getItem(PREFIX + key); } catch { return null; }
}

export function writeDraft(key: string, text: string | null) {
  if (!storageAllowed()) return;
  try { if (text === null) sessionStorage.removeItem(PREFIX + key); else sessionStorage.setItem(PREFIX + key, text); } catch { /* Storage full or disabled: the draft lives in memory only. */ }
}

/** A text field's value, kept across unmounts while it differs from `initial`. `kept`: it came back from storage.
 * `clear()` drops the draft (after saving, or Discard). `key` null keeps nothing (the source is still loading). */
export function useKeptDraft(key: string | null, initial: string) {
  const [text, setText] = useState(() => (key && readDraft(key)) ?? initial);
  const [kept] = useState(() => !!key && readDraft(key) !== null && readDraft(key) !== initial);
  const keyRef = useRef(key);
  keyRef.current = key;
  useEffect(() => {
    if (!key) return;
    writeDraft(key, text === initial ? null : text);
  }, [key, text, initial]);
  const clear = () => { if (keyRef.current) writeDraft(keyRef.current, null); };
  return { text, setText, kept, clear };
}

/** A short version tag of a text (FNV-1a), for draft keys of records without a version number. */
export function versionOf(text: string): string {
  let h = 0x811c9dc5;
  for (let i = 0; i < text.length; i++) { h ^= text.charCodeAt(i); h = Math.imul(h, 0x01000193); }
  return (h >>> 0).toString(36);
}
