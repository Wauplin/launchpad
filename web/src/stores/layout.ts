// Persistent session panes, saved arrangements and pins, one set per workspace (see workspace.ts).

import { create } from "zustand";
import { persist } from "zustand/middleware";
import { useWorkspace } from "./workspace";

/** All projects keep the original key. */
const storageKey = (workspace: string | null) => (workspace ? `launchpad.layout.${workspace}` : "launchpad.layout");

export type Layout = 1 | 2 | 4;
export type AgentGroup = "Cron" | "Done" | "Running" | "Scheduled" | "Archived";

type SavedLayout = { name: string; layout: Layout; panes: (string | null)[]; focused: number };

type LayoutStore = {
  pinned: string[];
  togglePin: (id: string) => void;
  saved: SavedLayout[];
  saveLayout: (name: string) => void;
  restoreLayout: (name: string) => void;
  deleteLayout: (name: string) => void;
  place: (id: string, pane: number) => void;
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

const defaults = { layout: 1 as Layout, pinned: [], saved: [], collapsedGroups: {}, panes: [null, null, null, null], focused: 0 };

export const useLayout = create<LayoutStore>()(
  persist(
    (set, get) => ({
      ...defaults,
      layout: 1,
      pinned: [],
      togglePin: (id) => set({ pinned: get().pinned.includes(id) ? get().pinned.filter((p) => p !== id) : [...get().pinned, id] }),
      saved: [],
      saveLayout: (name) => {
        name = name.trim().slice(0, 50);
        if (!name) return;
        const { layout, panes, focused, saved } = get();
        set({ saved: [...saved.filter((s) => s.name !== name), { name, layout, panes: [...panes], focused }].slice(-12) });
      },
      restoreLayout: (name) => { const s = get().saved.find((s) => s.name === name); if (s) set({ layout: s.layout, panes: [...s.panes], focused: s.focused }); },
      deleteLayout: (name) => set({ saved: get().saved.filter((s) => s.name !== name) }),
      place: (id, pane) => {
        const { panes, layout } = get();
        if (pane < 0 || pane >= layout) return;
        const next = [...panes];
        const from = next.indexOf(id);
        if (from >= 0) next[from] = next[pane];
        next[pane] = id;
        set({ panes: next, focused: pane });
      },
      collapsedGroups: {},
      toggleGroup: (group) => set({ collapsedGroups: { ...get().collapsedGroups, [group]: !get().collapsedGroups[group] } }),
      panes: [null, null, null, null],
      focused: 0,
      setLayout: (layout) => set({ layout, focused: Math.min(get().focused, layout - 1) }),
      focus: (focused) => { if (focused >= 0 && focused < get().layout) set({ focused }); },
      show: (id) => {
        const { panes, layout, focused } = get();
        const at = panes.slice(0, layout).indexOf(id);
        if (at >= 0) return set({ focused: at });
        get().place(id, focused);
      },
      clearSession: (id) => set({ panes: get().panes.map((p) => (p === id ? null : p)) }),
    }),
    { name: storageKey(useWorkspace.getState().current) },
  ),
);

/** Loads the panes of another workspace when this window switches. */
useWorkspace.subscribe((s, prev) => {
  if (s.current === prev.current) return;
  const name = storageKey(s.current);
  useLayout.persist.setOptions({ name });
  if (localStorage.getItem(name)) void useLayout.persist.rehydrate();
  else useLayout.setState(defaults);
});
