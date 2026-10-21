// Persistent session selection, pins and terminal dock, one set per workspace (see workspace.ts).

import { create } from "zustand";
import { persist } from "zustand/middleware";
import { useWorkspace } from "./workspace";

/** All workspaces keep the original key. */
const storageKey = (workspace: string | null) => (workspace ? `launchpad.layout.${workspace}` : "launchpad.layout");

export type Dock = { open: boolean; height: number; active: string | null };

export type AgentGroup = "Needs you" | "Cron" | "Done" | "Running" | "Quota paused" | "Stopped" | "Interrupted" | "Failed" | "In terminal" | "Scheduled" | "Archived";

type LayoutStore = {
  pinned: string[];
  togglePin: (id: string) => void;
  collapsedGroups: Partial<Record<AgentGroup, boolean>>;
  toggleGroup: (group: AgentGroup) => void;
  selectedSession: string | null;
  show: (id: string) => void;
  clearSession: (id: string) => void;
  /** The terminal dock: visibility, height and the active tab. */
  dock: Dock;
  setDock: (dock: Partial<Dock>) => void;
};

/** Groups collapsed until you expand them. */
const collapsedByDefault: AgentGroup[] = ["Cron", "Scheduled"];
export const isCollapsed = (groups: Partial<Record<AgentGroup, boolean>>, group: AgentGroup) => groups[group] ?? collapsedByDefault.includes(group);

const defaultDock: Dock = { open: false, height: 320, active: null };
const defaults = { pinned: [], collapsedGroups: {}, selectedSession: null, dock: defaultDock };

export const useLayout = create<LayoutStore>()(
  persist(
    (set, get) => ({
      ...defaults,
      togglePin: (id) => { refresh(); set({ pinned: get().pinned.includes(id) ? get().pinned.filter((p) => p !== id) : [...get().pinned, id] }); },
      toggleGroup: (group) => { refresh(); set({ collapsedGroups: { ...get().collapsedGroups, [group]: !isCollapsed(get().collapsedGroups, group) } }); },
      show: (selectedSession) => { refresh(); set({ selectedSession }); },
      clearSession: (id) => { refresh(); if (get().selectedSession === id) set({ selectedSession: null }); },
      setDock: (dock) => { refresh(); set({ dock: { ...get().dock, ...dock } }); },
    }),
    {
      name: storageKey(useWorkspace.getState().current),
      merge: (persisted, current) => {
        const { pinned, collapsedGroups, selectedSession, dock } = { ...defaults, ...(persisted as Partial<LayoutStore>) };
        // Preserve heights saved by the former S/M/L controls.
        const { size, ...savedDock } = dock as Partial<Dock> & { size?: "small" | "medium" | "large" };
        const height = savedDock.height ?? (size ? { small: 200, medium: 320, large: 500 }[size] : undefined);
        return { ...current, pinned, collapsedGroups, selectedSession, dock: { ...defaultDock, ...savedDock, height: typeof height === "number" && Number.isFinite(height) ? height : defaultDock.height } };
      },
    },
  ),
);

/** Loads the selected session and pins of another workspace when this window switches. */
useWorkspace.subscribe((s, prev) => {
  if (s.current === prev.current) return;
  const name = storageKey(s.current);
  useLayout.persist.setOptions({ name });
  if (localStorage.getItem(name)) void useLayout.persist.rehydrate();
  else useLayout.setState(defaults);
});

function refresh() { void useLayout.persist.rehydrate(); }
window.addEventListener("storage", (ev) => { if (ev.key === storageKey(useWorkspace.getState().current)) refresh(); });
