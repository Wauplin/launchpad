// The Ask panel: a bot's chat (features/chat/BotChat.tsx) beside the page, opened by Ask in the top bar. It shows
// the bot the field sent to; null means the workspace's default (`askTarget`). Not saved: each window opens closed.

import { create } from "zustand";

type AskPanel = {
  open: boolean;
  botId: string | null;
  show: (botId: string | null) => void;
  close: () => void;
};

export const useAskPanel = create<AskPanel>((set) => ({
  open: false,
  botId: null,
  show: (botId) => set({ open: true, botId }),
  close: () => set({ open: false }),
}));
