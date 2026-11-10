// "Your next message starts a new session" of a bot's thread, per bot, for this window: the one place that holds
// it. The bot's ⋯ → Start a new session… sets it with the owner's reason (laptop and phone); the message box shows
// it with Cancel and sends it with the next message. It holds only for the session that was current when it was set:
// once another session is current (the new one started, or a profile change rotated the thread), it no longer applies.

import { create } from "zustand";

/** A new session the owner asked for: the reason shows in the chat's divider. `after`: the session it replaces. */
export type NewSessionRequest = { reason: string; after: string | null };

export const useNextSession = create<{ bots: ReadonlyMap<string, NewSessionRequest>; set: (bot: string, request: NewSessionRequest | null) => void }>((set) => ({
  bots: new Map(),
  set: (bot, request) => set((s) => { const next = new Map(s.bots); if (request) next.set(bot, request); else next.delete(bot); return { bots: next }; }),
}));

/** The bot's armed new session while it still applies to `current` (the thread's current session), else null. */
export function useArmedSession(bot: string, current: string | null): NewSessionRequest | null {
  const request = useNextSession((s) => s.bots.get(bot) ?? null);
  return request && request.after === current ? request : null;
}
