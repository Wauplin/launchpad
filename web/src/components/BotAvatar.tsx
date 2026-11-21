import { memo, useEffect, useState } from "react";
import { create } from "zustand";
import { Icon, type IconName } from "@/components/Icon";
import { isDemo } from "@/demo/mode";
import { daemonAsset } from "@/lib/connection";
import { useBots } from "@/lib/bots";
import { type Identity, type IdentityColor, avatarPath, identityIndex, identityOfName, initialOf, nameKey } from "@/lib/identity";

// Full class names, so Tailwind sees them (lib/identity.ts picks the color).
const SOFT: Record<IdentityColor, string> = {
  teal: "bg-id-teal-soft text-id-teal", blue: "bg-id-blue-soft text-id-blue", violet: "bg-id-violet-soft text-id-violet", rose: "bg-id-rose-soft text-id-rose",
  amber: "bg-id-amber-soft text-id-amber", green: "bg-id-green-soft text-id-green", cyan: "bg-id-cyan-soft text-id-cyan", rust: "bg-id-rust-soft text-id-rust",
  indigo: "bg-id-indigo-soft text-id-indigo", magenta: "bg-id-magenta-soft text-id-magenta", olive: "bg-id-olive-soft text-id-olive", slate: "bg-id-slate-soft text-id-slate",
  orange: "bg-id-orange-soft text-id-orange", plum: "bg-id-plum-soft text-id-plum", lime: "bg-id-lime-soft text-id-lime", red: "bg-id-red-soft text-id-red",
};
const EDGE: Record<IdentityColor, string> = {
  teal: "bg-id-teal", blue: "bg-id-blue", violet: "bg-id-violet", rose: "bg-id-rose", amber: "bg-id-amber", green: "bg-id-green", cyan: "bg-id-cyan", rust: "bg-id-rust",
  indigo: "bg-id-indigo", magenta: "bg-id-magenta", olive: "bg-id-olive", slate: "bg-id-slate", orange: "bg-id-orange", plum: "bg-id-plum", lime: "bg-id-lime", red: "bg-id-red",
};
const INK: Record<IdentityColor, string> = {
  teal: "text-id-teal", blue: "text-id-blue", violet: "text-id-violet", rose: "text-id-rose", amber: "text-id-amber", green: "text-id-green", cyan: "text-id-cyan", rust: "text-id-rust",
  indigo: "text-id-indigo", magenta: "text-id-magenta", olive: "text-id-olive", slate: "text-id-slate", orange: "text-id-orange", plum: "text-id-plum", lime: "text-id-lime", red: "text-id-red",
};

type Index = ReturnType<typeof identityIndex>;
/** Every bot's identity (all workspaces), filled from the bots list by `BotIdentitySync` in both shells, so a name
 * alone (a channel member, a message's sender, an edge) finds its bot's chosen color and avatar. */
const useIdentities = create<Index>(() => ({ byId: new Map(), byName: new Map() }));

/** Keeps the identities in step with the bots list; mounted once per shell. */
export function BotIdentitySync() {
  const bots = useBots().data;
  useEffect(() => { if (bots) useIdentities.setState(identityIndex(bots)); }, [bots]);
  return null;
}

/** A bot's identity: by ID when known, else by name; a name no bot has gets its default. */
export const identityFor = (name: string, id?: string | null, s: Index = useIdentities.getState()): Identity =>
  (id ? s.byId.get(id) : undefined) ?? s.byName.get(nameKey(name)) ?? identityOfName(name);

/** The same, as a hook: re-renders when the bot's identity changes. */
export const useBotIdentity = (name: string, id?: string | null): Identity => {
  const s = useIdentities();
  return identityFor(name, id, s);
};

/** Re-renders a component that colors names with `botInk`/`botEdge` when identities change. */
export const useIdentitiesVersion = () => useIdentities();

/** A bot's mark colors (soft background and ink), the same for its name everywhere. */
export const botTone = (name: string, id?: string | null) => SOFT[identityFor(name, id).color];
/** The ink of a bot's name, matching its mark. */
export const botInk = (name: string, id?: string | null) => INK[identityFor(name, id).color];
/** The fill of a bot's thin edge on a message bubble (who wrote it, who got it), matching its mark. */
export const botEdge = (name: string, id?: string | null) => EDGE[identityFor(name, id).color];
/** Classes of a color itself (the settings' preview of an unsaved choice). */
export const toneClass = (c: IdentityColor) => SOFT[c];
export const inkClass = (c: IdentityColor) => INK[c];

/** Avatar URLs that failed in this window: they show the letter mark until the next load. */
const FAILED = new Set<string>();

/** A bot's mark: its DiceBear bottts robot (Home's cached copy) on a square in the bot's color; the first letter of
 * its name while the avatar loads nothing (demo, offline Home, DiceBear down). `identity` overrides the lookup (the
 * settings' preview). */
export const BotAvatar = memo(function BotAvatar({ name, id, size = 28, identity }: { name: string; id?: string | null; size?: number; identity?: Identity }) {
  const found = useBotIdentity(name, id);
  const who = identity ?? found;
  const src = isDemo ? null : daemonAsset(avatarPath(who.seed));
  const [failed, setFailed] = useState(() => !src || FAILED.has(src));
  useEffect(() => { setFailed(!src || FAILED.has(src)); }, [src]);
  return <span className={`grid flex-none place-items-center overflow-hidden rounded-card font-semibold ${SOFT[who.color]}`} style={{ width: size, height: size, fontSize: Math.round(size * 0.46) }} aria-hidden>
    {failed || !src ? initialOf(who.name)
      : <img src={src} alt="" width={size} height={size} loading="lazy" decoding="async" draggable={false} className="block size-full p-[8%]" onError={() => { FAILED.add(src); setFailed(true); }} />}
  </span>;
});

/** Your mark in chats: a round "Y" (people are round, bots square). */
export function OwnerAvatar({ size = 28 }: { size?: number }) {
  return <span className="grid flex-none place-items-center rounded-full bg-ink-2 font-semibold text-paper" style={{ width: size, height: size, fontSize: Math.round(size * 0.46) }} aria-hidden>Y</span>;
}

/** The mark of a sender that is not a bot (an agent, a hook, a channel, Launchpad): its icon on a grey square. */
export function SourceAvatar({ icon, size = 28 }: { icon: IconName; size?: number }) {
  return <span className="grid flex-none place-items-center rounded-card bg-sunk text-ink-3" style={{ width: size, height: size }} aria-hidden><Icon name={icon} size={Math.round(size * 0.5)} /></span>;
}
