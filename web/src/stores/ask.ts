// The Ask panel: a bot's chat (features/chat/BotChat.tsx) beside the page, opened by Ask in the top bar. It shows
// the bot the field sent to; null means the workspace's default (`askTarget`), and the thread of the last message
// (null: the thread the owner looked at last, `landingThread`; `new`: New thread). Not saved: each window opens closed.

import { create } from "zustand";

type AskPanel = {
  open: boolean;
  botId: string | null;
  thread: string | null;
  show: (botId: string | null, thread?: string | null) => void;
  setThread: (thread: string | null) => void;
  close: () => void;
};

export const useAskPanel = create<AskPanel>((set) => ({
  open: false,
  botId: null,
  thread: null,
  show: (botId, thread = null) => set({ open: true, botId, thread }),
  setThread: (thread) => set({ thread }),
  close: () => set({ open: false }),
}));
