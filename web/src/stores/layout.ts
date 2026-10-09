// Persistent session selection and pins, one set per workspace (see workspace.ts).

import { create } from "zustand";
import { persist } from "zustand/middleware";
import { useWorkspace } from "./workspace";

/** All projects keep the original key. */
const storageKey = (workspace: string | null) => (workspace ? `launchpad.layout.${workspace}` : "launchpad.layout");

export type AgentGroup = "Cron" | "Done" | "Running" | "Quota paused" | "Scheduled" | "Archived";

type LayoutStore = {
  pinned: string[];
  togglePin: (id: string) => void;
  collapsedGroups: Partial<Record<AgentGroup, boolean>>;
  toggleGroup: (group: AgentGroup) => void;
  selectedSession: string | null;
  show: (id: string) => void;
  clearSession: (id: string) => void;
};

const defaults = { pinned: [], collapsedGroups: {}, selectedSession: null };

export const useLayout = create<LayoutStore>()(
  persist(
    (set, get) => ({
      ...defaults,
      togglePin: (id) => set({ pinned: get().pinned.includes(id) ? get().pinned.filter((p) => p !== id) : [...get().pinned, id] }),
      toggleGroup: (group) => set({ collapsedGroups: { ...get().collapsedGroups, [group]: !get().collapsedGroups[group] } }),
      show: (selectedSession) => set({ selectedSession }),
      clearSession: (id) => { if (get().selectedSession === id) set({ selectedSession: null }); },
    }),
    {
      name: storageKey(useWorkspace.getState().current),
      merge: (persisted, current) => {
        const { pinned, collapsedGroups, selectedSession } = { ...defaults, ...(persisted as Partial<LayoutStore>) };
        return { ...current, pinned, collapsedGroups, selectedSession };
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
