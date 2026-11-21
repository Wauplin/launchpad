// Who wrote a message, at a glance: each bot keeps one color and one avatar everywhere (chats, channels, lists).
// Both come from its name unless bot.toml names them (`color`, one of `IDENTITY_COLORS`, never a hex code;
// `avatar_seed`), so every window agrees. The name always shows too: color is never the only sign. Pure, so it is
// tested (tests/chatRows.test.mjs).

/** The identity colors (`--color-id-<name>` and `--color-id-<name>-soft` in styles.css, readable in both themes).
 * Keep the order and names in step with `IDENTITY_COLORS` of crates/core/src/bots.rs. */
export const IDENTITY_COLORS = ["teal", "blue", "violet", "rose", "amber", "green", "cyan", "rust", "indigo", "magenta", "olive", "slate", "orange", "plum", "lime", "red"] as const;
export type IdentityColor = (typeof IDENTITY_COLORS)[number];

/** How many identity colors the theme has. */
export const IDENTITY_TONES = IDENTITY_COLORS.length;

/** The color index of a name: the same for the same name (case and outer spaces aside), spread over the palette. */
export function identityTone(name: string): number {
  let h = 0x811c9dc5;
  for (const c of name.trim().toLowerCase()) {
    h ^= c.codePointAt(0) ?? 0;
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h % IDENTITY_TONES;
}

export const isIdentityColor = (c: string | null | undefined): c is IdentityColor => !!c && (IDENTITY_COLORS as readonly string[]).includes(c);

/** The first letter of a name, for its mark when the avatar is not available. */
export const initialOf = (name: string, fallback = "B") => ([...name.trim()][0] ?? fallback).toUpperCase();

/** What identity needs of a bot (a `Bot` fits). */
export type IdentityBot = { id: string; created_at?: number; settings: { name: string; avatar_seed?: string | null; color?: string | null } };
export type Identity = { name: string; color: IdentityColor; seed: string };

const key = (name: string) => name.trim().toLowerCase();

/** The avatar seed a bot gets by default: its name, plus a stable suffix (from its ID) when an older bot of `all`
 * has the same name, so two bots never share a robot. */
export function defaultSeed(bot: IdentityBot, all: readonly IdentityBot[] = []): string {
  const name = bot.settings.name.trim() || "Bot";
  const older = all.some((o) => o.id !== bot.id && key(o.settings.name) === key(name) && ((o.created_at ?? 0) < (bot.created_at ?? 0) || ((o.created_at ?? 0) === (bot.created_at ?? 0) && o.id < bot.id)));
  return older ? `${name} ${bot.id.replace(/[^0-9a-z]/gi, "").slice(0, 6)}` : name;
}

/** A bot's identity: bot.toml's color and seed when set (an unknown color counts as unset), else from its name. */
export function identityOf(bot: IdentityBot, all: readonly IdentityBot[] = []): Identity {
  const name = bot.settings.name;
  const color = isIdentityColor(bot.settings.color) ? bot.settings.color : IDENTITY_COLORS[identityTone(name)];
  const seed = bot.settings.avatar_seed?.trim() || defaultSeed(bot, all);
  return { name, color, seed };
}

/** The identity of a name that is not a known bot (a deleted bot, a sender name only). */
export const identityOfName = (name: string): Identity => ({ name, color: IDENTITY_COLORS[identityTone(name)], seed: name.trim() || "Bot" });

/** Identities by bot ID and by name (lowercase; the oldest bot of a name wins). */
export function identityIndex(bots: readonly IdentityBot[]): { byId: Map<string, Identity>; byName: Map<string, Identity> } {
  const byId = new Map<string, Identity>(), byName = new Map<string, Identity>();
  const oldest = [...bots].sort((a, b) => (a.created_at ?? 0) - (b.created_at ?? 0) || (a.id < b.id ? -1 : 1));
  for (const b of oldest) {
    const id = identityOf(b, bots);
    byId.set(b.id, id);
    if (!byName.has(key(b.settings.name))) byName.set(key(b.settings.name), id);
  }
  return { byId, byName };
}

export const nameKey = key;

/** Another color than `current` (Suggest another color): `random` in [0, 1). */
export function nextColor(current: string, random = Math.random()): IdentityColor {
  const others = IDENTITY_COLORS.filter((c) => c !== current);
  return others[Math.min(others.length - 1, Math.floor(random * others.length))];
}

/** A new random seed for the avatar (Shuffle): short words are easier to keep. */
export function randomSeed(random: () => number = Math.random): string {
  const chars = "abcdefghijkmnpqrstuvwxyz23456789";
  return Array.from({ length: 8 }, () => chars[Math.floor(random() * chars.length)]).join("");
}

/** Home's cached copy of DiceBear's bottts avatar of a seed. */
export const avatarPath = (seed: string) => `/api/avatars/bottts?seed=${encodeURIComponent(seed.trim())}`;
