import { botInitial } from "@/lib/bots";

/** A bot's mark: the first letter of its name on a teal square. */
export function BotAvatar({ name, size = 28 }: { name: string; size?: number }) {
  return <span className="grid flex-none place-items-center rounded-card bg-teal-soft font-semibold text-teal-ink" style={{ width: size, height: size, fontSize: Math.round(size * 0.46) }} aria-hidden>{botInitial(name)}</span>;
}
