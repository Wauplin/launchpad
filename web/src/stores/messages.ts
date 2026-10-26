// Unsent text of each session's message box. Kept outside the box, so the code viewer can add a
// `path:405-411` reference to a session and the text survives switching views. Persisted in this
// browser's local storage, so a reload (or a phone discarding the tab) keeps unsent replies.
// The presets chosen for the next message are kept the same way.

import { create } from "zustand";
import { persist } from "zustand/middleware";

type Drafts = {
  drafts: Record<string, string>;
  /** Preset IDs to append to the next message of each session. */
  presets: Record<string, string[]>;
  set: (id: string, text: string) => void;
  setPresets: (id: string, presets: string[]) => void;
  /** Adds text after what is already typed. */
  append: (id: string, text: string) => void;
};

/** Empty boxes are not stored, so the saved map only holds sessions with unsent text. */
function withText(drafts: Record<string, string>, id: string, text: string): Record<string, string> {
  const next = { ...drafts };
  if (text) next[id] = text;
  else delete next[id];
  return next;
}

export const useMessageDrafts = create<Drafts>()(
  persist(
    (set, get) => ({
      drafts: {},
      presets: {},
      set: (id, text) => { void useMessageDrafts.persist.rehydrate(); set({ drafts: withText(get().drafts, id, text) }); },
      setPresets: (id, presets) => {
        void useMessageDrafts.persist.rehydrate();
        const next = { ...get().presets };
        if (presets.length) next[id] = presets;
        else delete next[id];
        set({ presets: next });
      },
      append: (id, text) => {
        void useMessageDrafts.persist.rehydrate();
        const cur = get().drafts[id] ?? "";
        set({ drafts: withText(get().drafts, id, cur && !/\s$/.test(cur) ? `${cur} ${text}` : `${cur}${text}`) });
      },
    }),
    { name: "launchpad-message-drafts", partialize: (s) => ({ drafts: s.drafts, presets: s.presets }) },
  ),
);

window.addEventListener("storage", (ev) => { if (ev.key === "launchpad-message-drafts") void useMessageDrafts.persist.rehydrate(); });
