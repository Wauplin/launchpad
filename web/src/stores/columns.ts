// Widths the owner gave the laptop layout's columns by dragging their dividers, on this device (browser storage;
// the demo keeps them in memory). A column without an entry has its default width.

import { create } from "zustand";
import { isDemo } from "../demo/mode.ts";
import { parseWidths, type ColumnName } from "../lib/columns.ts";

const key = "launchpad.columns";
type Widths = Partial<Record<ColumnName, number>>;

const read = (): Widths => {
  if (isDemo) return {};
  try { return parseWidths(localStorage.getItem(key)); } catch { return {}; }
};

export const useColumns = create<{ widths: Widths; set: (name: ColumnName, width: number | null) => void }>((set, get) => ({
  widths: read(),
  set: (name, width) => {
    const widths = { ...get().widths };
    if (width === null) delete widths[name];
    else widths[name] = width;
    set({ widths });
    if (!isDemo) try { localStorage.setItem(key, JSON.stringify(widths)); } catch { /* This window only. */ }
  },
}));

// Other windows of this browser follow.
window.addEventListener("storage", (event) => {
  if (event.storageArea === localStorage && (event.key === key || event.key === null)) useColumns.setState({ widths: read() });
});
