// Sidebar groups (Bots) open or collapsed, and the laptop sidebar narrowed to icons, on this device. The demo never writes browser storage.

import { create } from "zustand";
import { isDemo } from "../demo/mode";

export type SidebarGroup = "bots";
/** `sidebar`: the whole laptop sidebar shows icons only. */
type Key = SidebarGroup | "sidebar";
type Collapsed = Partial<Record<Key, boolean>>;
const key = "launchpad.sidebar";

function read(): Collapsed {
  if (isDemo) return {};
  try { return JSON.parse(localStorage.getItem(key) ?? "{}") as Collapsed; } catch { return {}; }
}

export const useSidebar = create<{ collapsed: Collapsed; toggle: (group: Key) => void }>((set, get) => ({
  collapsed: read(),
  toggle: (group) => {
    const collapsed = { ...get().collapsed, [group]: !get().collapsed[group] };
    set({ collapsed });
    if (!isDemo) try { localStorage.setItem(key, JSON.stringify(collapsed)); } catch { /* This window only. */ }
  },
}));

// Other windows of this browser share the choice.
window.addEventListener("storage", (event) => {
  if (event.storageArea === localStorage && (event.key === key || event.key === null)) useSidebar.setState({ collapsed: read() });
});
