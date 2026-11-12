// Demo routes: bot channels and DMs (`/api/channels`, Home's `launchpad_channels::channel`). Your messages post and
// record their deliveries like Home (mentions and members in mode all, a paused channel wakes nobody), but no bot
// actually answers. Counts are computed from the records like Home's `channel::view`. New channels and member changes
// (the Members panel, accepted channel and join cards) follow Home's checks.

import type { Channel } from "@/gen/Channel";
import type { ChannelDelivery } from "@/gen/ChannelDelivery";
import type { ChannelMessage } from "@/gen/ChannelMessage";
import type { ChannelPage } from "@/gen/ChannelPage";
import { clock, DemoError, publish, route } from "../daemon";
import { world } from "../world";
import type { ChannelMemberMode } from "@/gen/ChannelMemberMode";
import type { ChannelRecord } from "../world/channels";

const HOUR = 3_600_000;
const recordOf = (id: string) => {
  const c = world.botChannels.find((x) => x.id === id);
  if (!c) throw new DemoError(404, `no channel ${id}`);
  return c;
};
const changed = (c: ChannelRecord) => publish("channels", "changed", { id: c.id, workspace_id: c.workspace_id });
const messagesOf = (id: string) => world.channelMessages.filter((m) => m.channel_id === id).sort((a, b) => a.id - b.id);

function view(c: ChannelRecord): Channel {
  const bots = new Map(world.bots.map((b) => [b.id, b]));
  const members = c.members.map((m) => ({ ...m, name: bots.get(m.bot_id)?.settings.name ?? "deleted bot", slug: bots.get(m.bot_id)?.slug ?? "" }));
  const msgs = messagesOf(c.id);
  const last = msgs.at(-1);
  const unread = msgs.filter((m) => m.id > c.owner_read);
  const since = Math.max(clock.now() - HOUR, c.reset_at);
  return {
    ...c, members,
    title: c.kind === "dm" ? members.map((m) => m.name).sort().join(" ↔ ") : `#${c.name}`,
    last_message: last?.id ?? null, last_at: last?.created_at ?? null,
    last_by: last ? (last.author.kind === "owner" ? "you" : last.author.name) : null,
    last_text: last ? last.text.split("\n").find((l) => l.trim())?.trim().slice(0, 160) ?? "" : null,
    unread: unread.length,
    attention: unread.filter((m) => m.author.kind !== "owner" && (m.ask_owner || m.mentions.includes("owner"))).length,
    hour_count: msgs.filter((m) => m.author.kind === "bot" && m.created_at >= since).length,
    last_hops: last?.hops ?? 0,
  };
}

function page(c: ChannelRecord, before: number | null, limit: number): ChannelPage {
  const all = messagesOf(c.id).filter((m) => before == null || m.id < before);
  const messages = all.slice(-limit);
  const ids = new Set(messages.map((m) => m.id));
  return { channel: view(c), messages, deliveries: world.channelDeliveries.filter((d) => ids.has(d.message_id)), more: all.length > messages.length };
}

route("GET", "/api/channels", ({ query }) => {
  const ws = query.get("workspace");
  return world.botChannels.filter((c) => !ws || c.workspace_id === ws).map(view).sort((a, b) => (b.last_at ?? b.created_at) - (a.last_at ?? a.created_at));
});
route("GET", "/api/channels/:id", ({ params, query }) => page(recordOf(params.id), query.get("before") ? Number(query.get("before")) : null, Number(query.get("limit") ?? 50)));

/** Your message: hop 0, delivered to the bots it mentions and to members in mode all; it resets the hourly count. */
route("POST", "/api/channels/:id/messages", ({ params, body }) => {
  const c = recordOf(params.id);
  const text = String(body?.text ?? "").trim();
  if (!text) throw new DemoError(400, "the message is empty");
  if (c.archived) throw new DemoError(400, "this channel is archived");
  const slugs = new Map(c.members.map((m) => [world.bots.find((b) => b.id === m.bot_id)?.slug ?? "", m.bot_id]));
  const mentions = [...new Set([...text.matchAll(/(?:^|[\s(])@([a-z0-9-]+)/gi)].map((x) => x[1].toLowerCase()).map((s) => (s === "owner" || s === "you" ? "owner" : slugs.get(s))).filter((x): x is string => !!x))];
  const id = Math.max(0, ...world.channelMessages.map((m) => m.id)) + 1;
  const now = clock.now();
  const m: ChannelMessage = { id, channel_id: c.id, author: { kind: "owner", id: "owner", name: "You" }, session_id: null, turn: null, text, mentions, reply_to: typeof body?.reply_to === "number" ? body.reply_to : null, hops: 0, ask_owner: false, created_at: now };
  world.channelMessages.push(m);
  c.reset_at = now;
  c.owner_read = id;
  let next = Math.max(0, ...world.channelDeliveries.map((d) => d.id));
  const deliveries: ChannelDelivery[] = c.members.filter((x) => (c.kind === "dm" || x.mode === "all" || mentions.includes(x.bot_id)) && x.mode !== "muted").map((x) => ({
    id: ++next, message_id: id, bot_id: x.bot_id, at: now, joined: null,
    ...(c.paused ? { state: "skipped" as const, reason: "channel paused", session_id: null } : { state: "sent" as const, reason: null, session_id: world.sessions.filter((s) => s.launch_spec.bot?.id === x.bot_id).at(-1)?.id ?? null }),
  }));
  world.channelDeliveries.push(...deliveries);
  changed(c);
  return { message: m, deliveries };
});
route("POST", "/api/channels/:id/read", ({ params, body }) => {
  const c = recordOf(params.id);
  c.owner_read = typeof body?.through === "number" ? body.through : Math.max(c.owner_read, ...messagesOf(c.id).map((m) => m.id));
  changed(c);
  return view(c);
});
const flag = (key: "paused" | "blocked" | "archived") => ({ params, body }: { params: Record<string, string>; body: any }) => {
  const c = recordOf(params.id);
  if (key === "blocked" && c.kind !== "dm") throw new DemoError(400, "only a DM can be blocked");
  c[key] = !!body?.[key];
  changed(c);
  return view(c);
};
route("POST", "/api/channels/:id/pause", flag("paused"));
route("POST", "/api/channels/:id/block", flag("blocked"));
route("POST", "/api/channels/:id/archive", flag("archived"));
/** Allow 10 more: the newest message's loop-guard skips go now (here: they are marked sent). */
route("POST", "/api/channels/:id/allow-more", ({ params }) => {
  const c = recordOf(params.id);
  c.allowance += 10;
  const newest = messagesOf(c.id).at(-1);
  for (const d of world.channelDeliveries) {
    if (d.message_id !== newest?.id || d.state !== "skipped" || !/^(hop|hourly) limit/.test(d.reason ?? "") || c.allowance <= 0) continue;
    c.allowance -= 1;
    Object.assign(d, { state: "sent", reason: null, at: clock.now() });
  }
  changed(c);
  return page(c, null, 50);
});
route("PATCH", "/api/channels/:id", ({ params, body }) => {
  const c = recordOf(params.id);
  if (c.kind === "dm") throw new DemoError(400, "a DM has no name of its own");
  const name = typeof body?.name === "string" ? body.name.trim() : c.name;
  if (!/^[a-z0-9][a-z0-9-]{0,39}$/.test(name)) throw new DemoError(400, "a channel name uses lowercase letters, digits and dashes (at most 40)");
  if (world.botChannels.some((x) => x.id !== c.id && x.workspace_id === c.workspace_id && x.name === name)) throw new DemoError(400, `#${name} already exists in this workspace`);
  c.name = name;
  if (typeof body?.purpose === "string") c.purpose = body.purpose.trim();
  changed(c);
  return view(c);
});
route("DELETE", "/api/channels/:id", ({ params }) => {
  const c = recordOf(params.id);
  world.botChannels = world.botChannels.filter((x) => x.id !== c.id);
  const gone = new Set(messagesOf(c.id).map((m) => m.id));
  world.channelMessages = world.channelMessages.filter((m) => m.channel_id !== c.id);
  world.channelDeliveries = world.channelDeliveries.filter((d) => !gone.has(d.message_id));
  changed(c);
  return { ok: true };
});

const MODES: ChannelMemberMode[] = ["all", "mentions", "digest", "muted"];
const modeOf = (v: unknown): ChannelMemberMode => {
  if (v == null) return "mentions";
  if (!MODES.includes(v as ChannelMemberMode)) throw new DemoError(400, "mode is all, mentions, digest or muted");
  return v as ChannelMemberMode;
};
const botOf = (workspace: string, key: string) => {
  const b = world.bots.find((x) => x.workspace_id === workspace && (x.id === key || x.slug === key.replace(/^@/, "")));
  if (!b) throw new DemoError(400, `no bot ${key} in this workspace`);
  return b;
};

/** A new named channel (`channel.create`, or an accepted channel card): the name is free in the workspace, archived
 * channels included; members by ID or slug, mode `mentions` unless named. */
export function createChannel(workspace: string, name: string, purpose: string, members: unknown[]): Channel {
  name = name.trim().replace(/^#/, "");
  if (!/^[a-z0-9][a-z0-9-]{0,39}$/.test(name)) throw new DemoError(400, "a channel name has lowercase letters, digits and dashes (at most 40), like release-train");
  const taken = world.botChannels.find((x) => x.workspace_id === workspace && x.name === name);
  if (taken) throw new DemoError(400, taken.archived ? `#${name} already exists in this workspace, archived: bring it back (Bots → Archived channels) or delete it first` : `#${name} already exists in this workspace`);
  const list: ChannelRecord["members"] = [];
  for (const m of members) {
    const key = typeof m === "string" ? m : String((m as { bot?: unknown }).bot ?? "");
    const b = botOf(workspace, key);
    if (!list.some((x) => x.bot_id === b.id)) list.push({ bot_id: b.id, mode: modeOf(typeof m === "string" ? null : (m as { mode?: unknown }).mode), last_read: 0 });
  }
  const now = clock.now();
  const c: ChannelRecord = { id: `demo-chan-${name}-${now}`, workspace_id: workspace, kind: "channel", name, purpose: purpose.split(/\s+/).join(" ").trim().slice(0, 200), paused: false, blocked: false, hop_limit: 3, hourly_limit: 30, allowance: 0, reset_at: 0, archived: false, created_by: "owner", created_at: now, owner_read: 0, members: list };
  world.botChannels.push(c);
  changed(c);
  return view(c);
}

/** Adds a bot of the channel's workspace or changes its mode; null removes it (Home's `channel::set_member`). */
export function setMember(id: string, bot: string, mode: ChannelMemberMode | null): Channel {
  const c = recordOf(id);
  if (c.kind === "dm") throw new DemoError(400, "a DM's members are its two bots");
  const b = botOf(c.workspace_id, bot);
  const m = c.members.find((x) => x.bot_id === b.id);
  if (mode == null) c.members = c.members.filter((x) => x.bot_id !== b.id);
  else if (m) m.mode = mode;
  else c.members.push({ bot_id: b.id, mode, last_read: 0 });
  changed(c);
  return view(c);
}

route("POST", "/api/channels", ({ body }) => createChannel(String(body?.workspace_id ?? ""), String(body?.name ?? ""), String(body?.purpose ?? ""), Array.isArray(body?.members) ? body.members : []));
route("GET", "/api/channels/:id/members/:bot", ({ params }) => ({ mode: recordOf(params.id).members.find((m) => m.bot_id === params.bot)?.mode ?? null }));
route("PUT", "/api/channels/:id/members/:bot", ({ params, body }) => setMember(params.id, params.bot, body?.mode == null ? null : modeOf(body.mode)));
route("POST", "/api/channels/:id/members/:bot/remove", ({ params }) => setMember(params.id, params.bot, null));
route("PUT", "/api/channels/:id/members/:bot/mode", ({ params, body }) => {
  const c = recordOf(params.id);
  if (!c.members.some((m) => m.bot_id === params.bot)) throw new DemoError(400, `this bot is not a member of #${c.name}: add it first`);
  return setMember(params.id, params.bot, modeOf(body?.mode));
});
