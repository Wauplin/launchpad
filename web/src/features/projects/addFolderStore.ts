// Open state of the Add folder dialog, kept apart so the sidebar does not load the dialog.

import { create } from "zustand";

/** What the dialog starts with: Settings → GitHub opens it on Clone from GitHub with a repository and a workspace. */
export type AddFolderPreset = { repo?: string; workspaceId?: string };

export const useAddFolder = create<{ open: boolean; preset: AddFolderPreset; show: (preset?: AddFolderPreset) => void; close: () => void }>((set) => ({
  open: false,
  preset: {},
  show: (preset = {}) => set({ open: true, preset }),
  close: () => set({ open: false, preset: {} }),
}));
