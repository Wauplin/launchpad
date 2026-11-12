// Pure helpers of bot channels (lib/channels.ts has the query): the lists' split and counts, a row's last line, the
// delivery lines under a message, the loop-guard footer and the `@` word being typed. Tested in tests/channels.test.mjs.

import type { Channel } from "@/gen/Channel";
import type { ChannelDelivery } from "@/gen/ChannelDelivery";
import type { ChannelMessage } from "@/gen/ChannelMessage";
import type { ChannelMemberMode } from "@/gen/ChannelMemberMode";

/** The workspace's channels and DMs that are not archived, newest activity first (Home's order), split by kind. */
export function channelsOf(all: Channel[] | undefined, workspaceId: string | null | undefined): { channels: Channel[]; dms: Channel[] } {
  const mine = (all ?? []).filter((c) => c.workspace_id === workspaceId && !c.archived);
  return { channels: mine.filter((c) => c.kind === "channel"), dms: mine.filter((c) => c.kind === "dm") };
}

/** The workspace's archived channels and DMs: the Archived list, where they come back or go for good. */
export function archivedOf(all: Channel[] | undefined, workspaceId: string | null | undefined): Channel[] {
  return (all ?? []).filter((c) => c.workspace_id === workspaceId && c.archived);
}

/** When a member bot is woken, in words (the Members panel and the New channel form). */
export const MODES: { id: ChannelMemberMode; label: string; hint: string }[] = [
  { id: "all", label: "Every message", hint: "Woken by every message" },
  { id: "mentions", label: "When mentioned", hint: "Woken only when a message mentions it" },
  { id: "digest", label: "Daily digest", hint: "One message a day at 08:00 listing what is new, and woken when mentioned" },
  { id: "muted", label: "Muted", hint: "Never woken; it can still read and post" },
];

/** Why a new channel cannot have this name (Home checks again): lowercase letters, digits and dashes, starting with
 * a letter or digit, at most 40, and not taken by a named channel of the workspace. An archived channel keeps its
 * name until it is deleted, so the message says where to find it. */
export function channelNameError(name: string, taken: Pick<Channel, "name" | "archived">[]): string | null {
  if (!/^[a-z0-9][a-z0-9-]{0,39}$/.test(name)) return "Lowercase letters, digits and dashes, starting with a letter or digit";
  const t = taken.find((c) => c.name === name);
  if (t?.archived) return `#${name} is an archived channel: bring it back or delete it under Archived channels to use its name`;
  if (t) return `#${name} already exists in this workspace`;
  return null;
}

/** The mark of a channel row: "#" for a named channel, "↔" for a DM. */
export const channelMark = (c: Pick<Channel, "kind">) => (c.kind === "dm" ? "↔" : "#");

/** A row's second line: "Maintainer: confirmed, #118 is not in 2.4", "You: …". */
export function channelLastLine(c: Channel): string {
  if (!c.last_text) return c.purpose || "No message yet";
  const by = c.last_by === "you" ? "You" : c.last_by;
  return by ? `${by}: ${c.last_text}` : c.last_text;
}

/** Why a bot was woken by a message: its mention, a DM, or its mode `all` (a subscription). */
export function wakeReason(kind: Channel["kind"], m: Pick<ChannelMessage, "mentions">, botId: string): "mention" | "DM" | "subscription" {
  if (m.mentions.includes(botId)) return "mention";
  return kind === "dm" ? "DM" : "subscription";
}

/** A skip by the loop guard (`delivery::loop_skip`): Allow 10 more delivers these again. */
export const loopSkip = (reason: string | null | undefined) => !!reason && (reason.startsWith("hop limit") || reason.startsWith("hourly limit"));

/** One muted line under a message per delivery. */
export type DeliveryLine =
  | { kind: "woke"; bot: string; botId: string; why: "mention" | "DM" | "subscription"; session: string | null }
  | { kind: "queued"; bot: string; botId: string; waiting: number; reason: string | null }
  | { kind: "skipped"; bot: string; botId: string; reason: string; loop: boolean }
  | { kind: "combined"; bot: string; botId: string; with: number | null; session: string | null }
  | { kind: "digest"; bot: string; botId: string; count: number; state: "sent" | "queued" | "skipped"; reason: string | null; session: string | null };

/** The delivery lines of each message, in delivery order. `waiting` counts the bot's deliveries of this channel that
 * still wait in its queue (the message and those combined with it). */
export function deliveryLines(kind: Channel["kind"], messages: ChannelMessage[], deliveries: ChannelDelivery[], nameOf: (botId: string) => string): Map<number, DeliveryLine[]> {
  const byId = new Map(deliveries.map((d) => [d.id, d]));
  const waits = (d: ChannelDelivery) => d.kind !== "digest" && (d.state === "queued" || d.state === "combined") && !d.session_id;
  const waiting = new Map<string, number>();
  for (const d of deliveries) if (waits(d)) waiting.set(d.bot_id, (waiting.get(d.bot_id) ?? 0) + 1);
  const msg = new Map(messages.map((m) => [m.id, m]));
  const out = new Map<number, DeliveryLine[]>();
  for (const d of deliveries) {
    const m = msg.get(d.message_id);
    if (!m) continue;
    const bot = nameOf(d.bot_id);
    // A daily digest shows under the newest message it lists.
    const line: DeliveryLine = d.kind === "digest"
      ? { kind: "digest", bot, botId: d.bot_id, count: d.count ?? 1, state: d.state === "skipped" ? "skipped" : d.state === "sent" || d.session_id ? "sent" : "queued", reason: d.reason, session: d.session_id }
      : d.state === "sent"
      ? { kind: "woke", bot, botId: d.bot_id, why: wakeReason(kind, m, d.bot_id), session: d.session_id }
      : d.state === "skipped"
        ? { kind: "skipped", bot, botId: d.bot_id, reason: d.reason ?? "no reason recorded", loop: loopSkip(d.reason) }
        : d.state === "combined"
          ? { kind: "combined", bot, botId: d.bot_id, with: (d.joined != null ? byId.get(d.joined)?.message_id : undefined) ?? null, session: d.session_id }
          : d.session_id
            ? { kind: "woke", bot, botId: d.bot_id, why: wakeReason(kind, m, d.bot_id), session: d.session_id }
            : { kind: "queued", bot, botId: d.bot_id, waiting: waiting.get(d.bot_id) ?? 1, reason: d.reason };
    out.set(m.id, [...(out.get(m.id) ?? []), line]);
  }
  return out;
}

/** The loop-guard footer: shown once bots answer each other (the newest message's hop is 2 or more) or a limit stopped
 * a delivery of the newest message. `stopped` names the limit. */
export function loopFooter(c: Channel, newest: ChannelMessage | undefined, deliveries: ChannelDelivery[]): { text: string; stopped: string | null } | null {
  const skip = newest ? deliveries.find((d) => d.message_id === newest.id && d.kind !== "digest" && d.state === "skipped" && loopSkip(d.reason)) : undefined;
  if (!skip && c.last_hops < 2) return null;
  const base = `Hop ${c.last_hops} of ${c.hop_limit} · ${c.hour_count} bot message${c.hour_count === 1 ? "" : "s"} this hour (limit ${c.hourly_limit})`;
  const allowance = c.allowance > 0 ? ` · ${c.allowance} more allowed` : "";
  const stopped = skip ? (skip.reason!.startsWith("hop limit") ? "The hop limit stopped a delivery" : "The hourly limit stopped a delivery") : null;
  return { text: base + allowance, stopped };
}

/** The `@` word being typed just before the caret, for mention suggestions: its start and the part typed. */
export function mentionAt(text: string, caret: number): { start: number; query: string } | null {
  const m = /(^|[\s(])@([a-z0-9-]*)$/i.exec(text.slice(0, caret));
  return m ? { start: caret - m[2].length - 1, query: m[2].toLowerCase() } : null;
}
