// Open state of the Add folder dialog, kept apart so the sidebar does not load the dialog.

import { create } from "zustand";

export const useAddFolder = create<{ open: boolean; show: () => void; close: () => void }>((set) => ({
  open: false,
  show: () => set({ open: true }),
  close: () => set({ open: false }),
}));
