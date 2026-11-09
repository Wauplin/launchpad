// The workspace shown in this window. A window always shows one workspace; null only until the workspaces load
// and `useWorkspaceSync` picks one.
//
// sessionStorage is per window, so several windows show different workspaces and each keeps its own after
// a reload. localStorage remembers the last choice for the next window. `?ws=<id or name>` (from
// notification links and Open in a new tab) wins and is removed from the address once read.

import { create } from "zustand";

const WINDOW_KEY = "launchpad.workspace";
const LAST_KEY = "launchpad.workspace.last";

type WorkspaceStore = {
  current: string | null;
  /** An id or name from the address, resolved once the workspaces load. */
  requested: string | null;
  switchTo: (id: string) => void;
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
const stored = sessionStorage.getItem(WINDOW_KEY) || localStorage.getItem(LAST_KEY) || null;

export const useWorkspace = create<WorkspaceStore>((set) => ({
  current: stored,
  requested,
  switchTo: (id) => {
    sessionStorage.setItem(WINDOW_KEY, id);
    localStorage.setItem(LAST_KEY, id);
    set({ current: id, requested: null });
  },
}));

/** Query string for the per-workspace composer draft. */
export const draftQuery = () => {
  const ws = useWorkspace.getState().current;
  return ws ? `?workspace=${encodeURIComponent(ws)}` : "";
};
