// Go to: paste a Launchpad link or short ID (`lp:chat/…`, a message's link, a folio section) and open its page at
// the exact place, switching the window's workspace when the object is of another one (the owner is never
// limited). Opened by the top bar's Go to button, More on the phone, or Ctrl+K / Cmd+K. Also installs the
// navigator that Markdown links and other references use (`lib/refActions.ts`).

import { useNavigate } from "@tanstack/react-router";
import { lazy, Suspense, useEffect } from "react";
import { create } from "zustand";
import { setRefNavigator } from "@/lib/refActions";

const GoToDialog = lazy(() => import("./GoToDialog").then((m) => ({ default: m.GoToDialog })));

/** Whether Go to is open (one dialog for the window). */
export const useGoTo = create<{ open: boolean; set: (open: boolean) => void }>((set) => ({ open: false, set: (open) => set({ open }) }));

/** Mounted once per shell: the router's navigation for references, and the Go to shortcut. */
export function GoToRoot() {
  const navigate = useNavigate();
  const open = useGoTo((s) => s.open);
  useEffect(() => {
    setRefNavigator((path) => void navigate({ href: path }));
    return () => setRefNavigator(null);
  }, [navigate]);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey) || e.altKey || e.shiftKey || e.key.toLowerCase() !== "k") return;
      // Terminals keep their own keys.
      if ((e.target as HTMLElement | null)?.closest?.(".xterm")) return;
      e.preventDefault();
      useGoTo.getState().set(true);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  return open ? <Suspense fallback={null}><GoToDialog onClose={() => useGoTo.getState().set(false)} /></Suspense> : null;
}
