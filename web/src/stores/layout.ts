// Persistent session selection, pins and terminal dock, one set per workspace (see workspace.ts).

import { create } from "zustand";
import { persist } from "zustand/middleware";
import { useWorkspace } from "./workspace";

/** All workspaces keep the original key. */
const storageKey = (workspace: string | null) => (workspace ? `launchpad.layout.${workspace}` : "launchpad.layout");

export type DockSize = "small" | "medium" | "large";
export type Dock = { open: boolean; size: DockSize; active: string | null };

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

const defaultDock: Dock = { open: false, size: "medium", active: null };
const defaults = { pinned: [], collapsedGroups: {}, selectedSession: null, dock: defaultDock };

export const useLayout = create<LayoutStore>()(
  persist(
    (set, get) => ({
      ...defaults,
      togglePin: (id) => set({ pinned: get().pinned.includes(id) ? get().pinned.filter((p) => p !== id) : [...get().pinned, id] }),
      toggleGroup: (group) => set({ collapsedGroups: { ...get().collapsedGroups, [group]: !isCollapsed(get().collapsedGroups, group) } }),
      show: (selectedSession) => set({ selectedSession }),
      clearSession: (id) => { if (get().selectedSession === id) set({ selectedSession: null }); },
      setDock: (dock) => set({ dock: { ...get().dock, ...dock } }),
    }),
    {
      name: storageKey(useWorkspace.getState().current),
      merge: (persisted, current) => {
        const { pinned, collapsedGroups, selectedSession, dock } = { ...defaults, ...(persisted as Partial<LayoutStore>) };
        return { ...current, pinned, collapsedGroups, selectedSession, dock: { ...defaultDock, ...dock } };
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
