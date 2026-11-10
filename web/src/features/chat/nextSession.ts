// "Next message starts a new session" of a bot's thread, per bot, for this window. The laptop sets it with the
// checkbox next to the message box; the phone with the bot's ⋯, and the chat shows it as a chip over the box.

import { create } from "zustand";

export const useNextSession = create<{ bots: ReadonlySet<string>; set: (bot: string, on: boolean) => void }>((set) => ({
  bots: new Set(),
  set: (bot, on) => set((s) => { const next = new Set(s.bots); if (on) next.add(bot); else next.delete(bot); return { bots: next }; }),
}));
