// A chat's mark and its bots' names, shared by the Chats list, the sidebar's pinned chats and the chat headers.

import { BotAvatar } from "@/components/BotAvatar";
import type { Bot } from "@/gen/Bot";
import { channelMark } from "@/lib/channels";
import type { Chat } from "@/lib/chatList";

/** A chat's mark: its first bot's square (a second bot's small square on its corner), or a channel's grey "#" or "↔". */
export function ChatMark({ chat, size = 18 }: { chat: Chat; size?: number }) {
  if (chat.kind === "channel") return <span className="grid flex-none place-items-center rounded-sm border border-line bg-sunk font-semibold text-ink-2" style={{ width: size, height: size, fontSize: Math.round(size * 0.62) }} aria-hidden>{channelMark(chat.channel)}</span>;
  const [first, second] = chat.participants;
  return <span className="relative flex-none" style={{ width: size, height: size }} aria-hidden>
    <BotAvatar name={first.settings.name} size={size} />
    {second && <span className="absolute -right-1 -bottom-1 rounded-card ring-2 ring-paper"><BotAvatar name={second.settings.name} size={Math.round(size * 0.6)} /></span>}
  </span>;
}

/** "Scout, Docs and Release": the bots of a chat in the order they joined. */
export const participantNames = (bots: readonly Bot[]) => bots.length <= 1 ? bots[0]?.settings.name ?? "" : `${bots.slice(0, -1).map((b) => b.settings.name).join(", ")} and ${bots.at(-1)!.settings.name}`;

