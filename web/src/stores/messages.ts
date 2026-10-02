// Unsent text of each session's message box. Kept outside the box, so the code viewer can add a
// `path:405-411` reference to a session and the text survives switching views.

import { create } from "zustand";

type Drafts = {
  drafts: Record<string, string>;
  set: (id: string, text: string) => void;
  /** Adds text after what is already typed. */
  append: (id: string, text: string) => void;
};

export const useMessageDrafts = create<Drafts>((set, get) => ({
  drafts: {},
  set: (id, text) => set({ drafts: { ...get().drafts, [id]: text } }),
  append: (id, text) => {
    const cur = get().drafts[id] ?? "";
    set({ drafts: { ...get().drafts, [id]: cur && !/\s$/.test(cur) ? `${cur} ${text}` : `${cur}${text}` } });
  },
}));
