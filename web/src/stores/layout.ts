// Agents workspace layout: 1, 2 or 4 panes, which session each shows, and the focused pane.

import { create } from "zustand";
import { persist } from "zustand/middleware";

export type Layout = 1 | 2 | 4;
export type AgentGroup = "Done" | "Running" | "Archived";

type LayoutStore = {
  layout: Layout;
  collapsedGroups: Partial<Record<AgentGroup, boolean>>;
  toggleGroup: (group: AgentGroup) => void;
  panes: (string | null)[];
  focused: number;
  setLayout: (l: Layout) => void;
  focus: (i: number) => void;
  /** Shows a session in the focused pane (or focuses the pane that already shows it). */
  show: (id: string) => void;
  clearSession: (id: string) => void;
};

export const useLayout = create<LayoutStore>()(
  persist(
    (set, get) => ({
      layout: 1,
      collapsedGroups: {},
      toggleGroup: (group) => set({ collapsedGroups: { ...get().collapsedGroups, [group]: !get().collapsedGroups[group] } }),
      panes: [null, null, null, null],
      focused: 0,
      setLayout: (layout) => set({ layout, focused: Math.min(get().focused, layout - 1) }),
      focus: (focused) => set({ focused }),
      show: (id) => {
        const { panes, layout, focused } = get();
        const at = panes.slice(0, layout).indexOf(id);
        if (at >= 0) return set({ focused: at });
        const next = [...panes];
        next[focused] = id;
        set({ panes: next });
      },
      clearSession: (id) => set({ panes: get().panes.map((p) => (p === id ? null : p)) }),
    }),
    { name: "launchpad.layout" },
  ),
);
