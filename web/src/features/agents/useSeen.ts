// Seen state for the "Needs you" group: a Done, Failed or Interrupted session shown in a focused window
// becomes seen (POST /api/sessions/{id}/seen, broadcast to every window).

import { useEffect, useState } from "react";
import type { Session } from "@/gen/Session";
import { api } from "@/lib/api";
import { isDone, isUnseen } from "@/lib/sessionStatus";

const focusedNow = () => document.hasFocus() && document.visibilityState === "visible";

export function useWindowFocused(): boolean {
  const [focused, setFocused] = useState(focusedNow);
  useEffect(() => {
    const update = () => setFocused(focusedNow());
    window.addEventListener("focus", update);
    window.addEventListener("blur", update);
    document.addEventListener("visibilitychange", update);
    return () => {
      window.removeEventListener("focus", update);
      window.removeEventListener("blur", update);
      document.removeEventListener("visibilitychange", update);
    };
  }, []);
  return focused;
}

/** Marks `s` seen once its Summary or Trace has been on screen in a focused window for a moment. */
export function useMarkSeen(s: Session | null | undefined, focused: boolean) {
  const due = !!s && focused && isUnseen(s) && (isDone(s) || s.status === "error" || s.status === "interrupted");
  const id = s?.id;
  const since = s?.status_since;
  useEffect(() => {
    if (!due || !id) return;
    const t = setTimeout(() => void api.post(`/api/sessions/${id}/seen`).catch(() => {}), 1500);
    return () => clearTimeout(t);
  }, [due, id, since]);
}
