// Page explanations show on the first visit on this device, then stay behind ⓘ. The visited pages are kept in
// browser storage (never in config.toml); the demo never writes browser storage, so it shows them once per load.

import { useEffect, useState } from "react";
import { isDemo } from "../demo/mode";
import { storageAllowed } from "./clientStorage";

const KEY = "launchpad.intros-seen";

function read(): Set<string> {
  if (isDemo) return new Set();
  try { return new Set(JSON.parse(localStorage.getItem(KEY) ?? "[]") as string[]); } catch { return new Set(); }
}

const seen = read();

/** `open` is true on the first visit of `id` (null: no explanation); `toggle` shows or hides it afterwards. */
export function useIntro(id: string | null): { open: boolean; toggle: () => void } {
  const [state, setState] = useState(() => ({ id, open: !!id && !seen.has(id) }));
  // A new page (id) starts from its own first-visit state; the same page keeps what the owner chose.
  let current = state;
  if (state.id !== id) {
    current = { id, open: !!id && !seen.has(id) };
    setState(current);
  }
  useEffect(() => {
    if (!id || seen.has(id)) return;
    seen.add(id);
    if (!isDemo && storageAllowed()) try { localStorage.setItem(KEY, JSON.stringify([...seen])); } catch { /* This window only. */ }
  }, [id]);
  return { open: current.open, toggle: () => setState((s) => ({ ...s, open: !s.open })) };
}
