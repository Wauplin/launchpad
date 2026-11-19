// The Chats list's state: sections collapsed on this device (browser storage; Archived collapsed until you open it),
// and the search and bot filter of this window (in memory, so they survive opening a chat and coming back on the
// phone). The demo never writes browser storage.

import { create } from "zustand";
import { isDemo } from "../demo/mode.ts";
import type { ChatSection } from "../lib/chatList.ts";

type Collapsed = Partial<Record<ChatSection, boolean>>;
const key = "launchpad.chat-sections";
const DEFAULTS: Collapsed = { Archived: true };

function read(): Collapsed {
  if (isDemo) return DEFAULTS;
  try { return { ...DEFAULTS, ...(JSON.parse(localStorage.getItem(key) ?? "{}") as Collapsed) }; } catch { return DEFAULTS; }
}

type ChatsStore = {
  collapsed: Collapsed;
  toggle: (section: ChatSection) => void;
  /** The search field's text, and whether it is open. */
  search: string;
  searchOpen: boolean;
  setSearch: (search: string, open?: boolean) => void;
  /** Bots that must all take part in a chat for it to show. */
  bots: string[];
  setBots: (bots: string[]) => void;
};

export const useChatsStore = create<ChatsStore>((set, get) => ({
  collapsed: read(),
  toggle: (section) => {
    const collapsed = { ...get().collapsed, [section]: !get().collapsed[section] };
    set({ collapsed });
    if (!isDemo) try { localStorage.setItem(key, JSON.stringify(collapsed)); } catch { /* This window only. */ }
  },
  search: "",
  searchOpen: false,
  setSearch: (search, open) => set({ search, searchOpen: open ?? get().searchOpen }),
  bots: [],
  setBots: (bots) => set({ bots }),
}));

// Other windows of this browser share the collapsed sections.
window.addEventListener("storage", (event) => {
  if (event.storageArea === localStorage && (event.key === key || event.key === null)) useChatsStore.setState({ collapsed: read() });
});
