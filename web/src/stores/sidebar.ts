// Sidebar groups (Bots) open or collapsed, and the laptop sidebar narrowed to icons, on this device. The demo never writes browser storage.

import { create } from "zustand";
import { isDemo } from "../demo/mode.ts";

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


type Orders = Record<string, string[]>;
const orderKey = "launchpad.sidebar.order";
function readOrders(): Orders {
  if (isDemo) return {};
  try {
    const value: unknown = JSON.parse(localStorage.getItem(orderKey) ?? "{}");
    if (!value || typeof value !== "object" || Array.isArray(value)) return {};
    return Object.fromEntries(Object.entries(value).filter(([, ids]) => Array.isArray(ids) && ids.every((id: unknown) => typeof id === "string")));
  } catch { return {}; }
}

/** Keep unseen rows in their original order, after the owner's saved rows. */
export function ordered<T extends { id: string }>(items: T[], ids: string[] = []): T[] {
  const ranks = new Map(ids.map((id, index) => [id, index]));
  return [...items].sort((a, b) => (ranks.get(a.id) ?? ids.length) - (ranks.get(b.id) ?? ids.length));
}

export const useSidebarOrder = create<{ orders: Orders; move: (list: string, ids: string[], from: string, to: string) => void }>((set, get) => ({
  orders: readOrders(),
  move: (list, ids, from, to) => {
    const start = ids.indexOf(from), end = ids.indexOf(to);
    if (start < 0 || end < 0 || start === end) return;
    const next = [...ids];
    next.splice(start, 1);
    next.splice(end, 0, from);
    const orders = { ...get().orders, [list]: next };
    set({ orders });
    if (!isDemo) try { localStorage.setItem(orderKey, JSON.stringify(orders)); } catch { /* This window only. */ }
  },
}));
window.addEventListener("storage", (event) => {
  if (event.storageArea === localStorage && (event.key === orderKey || event.key === null)) useSidebarOrder.setState({ orders: readOrders() });
});
