// The workspace shown in this window (ARCHITECTURE.md, 9.1.2). null means all projects.
//
// sessionStorage is per window, so several windows show different workspaces and each keeps its own after
// a reload. localStorage remembers the last choice for the next window. `?ws=<id or name>` (from
// `launchpad-desktop --workspace`) wins and is removed from the address once read.

import { create } from "zustand";

const WINDOW_KEY = "launchpad.workspace";
const LAST_KEY = "launchpad.workspace.last";

type WorkspaceStore = {
  current: string | null;
  /** An id or name from the address, resolved once the workspaces load. */
  requested: string | null;
  switchTo: (id: string | null) => void;
};

function takeRequested(): string | null {
  const params = new URLSearchParams(location.search);
  const ws = params.get("ws");
  if (ws === null) return null;
  params.delete("ws");
  const rest = params.toString();
  history.replaceState(history.state, "", location.pathname + (rest ? `?${rest}` : "") + location.hash);
  return ws;
}

const requested = takeRequested();
const stored = sessionStorage.getItem(WINDOW_KEY) ?? localStorage.getItem(LAST_KEY);

export const useWorkspace = create<WorkspaceStore>((set) => ({
  current: requested === null && stored ? stored : null,
  requested,
  switchTo: (id) => {
    sessionStorage.setItem(WINDOW_KEY, id ?? "");
    localStorage.setItem(LAST_KEY, id ?? "");
    set({ current: id, requested: null });
  },
}));

/** The desktop shell's `--workspace` on a running window navigates to `?ws=…` without reloading. */
window.addEventListener("popstate", () => {
  const ws = takeRequested();
  if (ws !== null) useWorkspace.setState({ requested: ws });
});

/** Query string for the per-workspace composer draft. */
export const draftQuery = () => {
  const ws = useWorkspace.getState().current;
  return ws ? `?workspace=${encodeURIComponent(ws)}` : "";
};
