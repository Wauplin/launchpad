import { Icon, type IconName } from "@/components/Icon";
import { identityTone, initialOf } from "@/lib/identity";

// Full class names, so Tailwind sees them (lib/identity.ts picks the index).
const SOFT = ["bg-id-0-soft text-id-0", "bg-id-1-soft text-id-1", "bg-id-2-soft text-id-2", "bg-id-3-soft text-id-3", "bg-id-4-soft text-id-4", "bg-id-5-soft text-id-5", "bg-id-6-soft text-id-6", "bg-id-7-soft text-id-7"];
const INK = ["text-id-0", "text-id-1", "text-id-2", "text-id-3", "text-id-4", "text-id-5", "text-id-6", "text-id-7"];

/** A bot's mark colors (soft background and ink), the same for its name everywhere. */
export const botTone = (name: string) => SOFT[identityTone(name)];
/** The ink of a bot's name, matching its mark. */
export const botInk = (name: string) => INK[identityTone(name)];

/** A bot's mark: the first letter of its name on a square in the bot's color. */
export function BotAvatar({ name, size = 28 }: { name: string; size?: number }) {
  return <span className={`grid flex-none place-items-center rounded-card font-semibold ${botTone(name)}`} style={{ width: size, height: size, fontSize: Math.round(size * 0.46) }} aria-hidden>{initialOf(name)}</span>;
}

/** Your mark in chats: a round "Y" (people are round, bots square). */
export function OwnerAvatar({ size = 28 }: { size?: number }) {
  return <span className="grid flex-none place-items-center rounded-full bg-ink-2 font-semibold text-paper" style={{ width: size, height: size, fontSize: Math.round(size * 0.46) }} aria-hidden>Y</span>;
}

/** The mark of a sender that is not a bot (an agent, a hook, a channel, Launchpad): its icon on a grey square. */
export function SourceAvatar({ icon, size = 28 }: { icon: IconName; size?: number }) {
  return <span className="grid flex-none place-items-center rounded-card bg-sunk text-ink-3" style={{ width: size, height: size }} aria-hidden><Icon name={icon} size={Math.round(size * 0.5)} /></span>;
}
