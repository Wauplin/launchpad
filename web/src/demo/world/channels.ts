// Bot channels part of the demo world (Home's `channel`, `channel_member`, `channel_message` and `channel_delivery`
// tables): #release-train, where the Release bot, the Maintainer and Docs (in mode digest) plan the weekly release
// until the hop limit stops a delivery and Release asks you (a Today row); #triage, where the Maintainer asks you twice; the DM Maintainer ↔ Release, which you joined; and
// #spring-sale, archived after the sale (Bots → Archived channels).
// The routes (routes/channels.ts) compute the counts Home adds (unread, attention, hourly count, last message).

import type { Channel } from "@/gen/Channel";
import type { ChannelDelivery } from "@/gen/ChannelDelivery";
import type { ChannelMemberMode } from "@/gen/ChannelMemberMode";
import type { ChannelMessage } from "@/gen/ChannelMessage";
import { clock } from "../daemon";
import { BOT, DOCS_BOT, RELEASE_BOT } from "./bots";
import { WS } from "./ids";

/** A channel as stored: the computed fields of `Channel` come from the routes. */
export type ChannelRecord = Pick<Channel, "id" | "workspace_id" | "kind" | "name" | "purpose" | "paused" | "blocked" | "hop_limit" | "hourly_limit" | "allowance" | "reset_at" | "archived" | "created_by" | "created_at" | "owner_read"> & {
  members: { bot_id: string; mode: ChannelMemberMode; last_read: number }[];
};

export const RELEASE_TRAIN = "demo-chan-release-train";
export const TRIAGE = "demo-chan-triage";
export const DM_MAINTAINER_RELEASE = "demo-chan-dm-maintainer-release";
export const SPRING_SALE = "demo-chan-spring-sale";

export function channelsWorld() {
  const base = { workspace_id: WS.acme, paused: false, blocked: false, hop_limit: 3, hourly_limit: 30, allowance: 0, archived: false, created_by: "owner" };
  const botChannels: ChannelRecord[] = [
    { ...base, id: RELEASE_TRAIN, kind: "channel", name: "release-train", purpose: "Plan and ship the weekly release", reset_at: clock.ago(60 * 3), created_at: clock.ago(60 * 24 * 6), owner_read: 42,
      members: [{ bot_id: RELEASE_BOT, mode: "all", last_read: 45 }, { bot_id: BOT, mode: "mentions", last_read: 43 }, { bot_id: DOCS_BOT, mode: "digest", last_read: 40 }] },
    { ...base, id: TRIAGE, kind: "channel", name: "triage", purpose: "Sort new storefront issues", reset_at: clock.ago(60 * 24), created_at: clock.ago(60 * 24 * 5), owner_read: 20,
      members: [{ bot_id: BOT, mode: "all", last_read: 22 }] },
    { ...base, id: DM_MAINTAINER_RELEASE, kind: "dm", name: `dm:${BOT}:${RELEASE_BOT}`, purpose: "", reset_at: clock.ago(25), created_at: clock.ago(60 * 24 * 2), owner_read: 33, created_by: `bot:${RELEASE_BOT}`,
      members: [{ bot_id: BOT, mode: "all", last_read: 34 }, { bot_id: RELEASE_BOT, mode: "all", last_read: 34 }] },
    { ...base, id: SPRING_SALE, kind: "channel", name: "spring-sale", purpose: "Prices and banners of the spring sale", archived: true, reset_at: clock.ago(60 * 24 * 40), created_at: clock.ago(60 * 24 * 50), owner_read: 11,
      members: [{ bot_id: BOT, mode: "mentions", last_read: 11 }, { bot_id: DOCS_BOT, mode: "all", last_read: 11 }] },
  ];
  const owner = { kind: "owner" as const, id: "owner", name: "You" };
  const release = { kind: "bot" as const, id: RELEASE_BOT, name: "Release" };
  const maintainer = { kind: "bot" as const, id: BOT, name: "Maintainer" };
  const msg = (id: number, channel_id: string, author: ChannelMessage["author"], minutesAgo: number, text: string, extra: Partial<ChannelMessage> = {}): ChannelMessage => ({
    id, channel_id, author, session_id: null, turn: null, text, mentions: [], reply_to: null, hops: author.kind === "owner" ? 0 : 1, ask_owner: false, created_at: clock.ago(minutesAgo), ...extra,
  });
  const channelMessages: ChannelMessage[] = [
    msg(10, SPRING_SALE, owner, 60 * 24 * 41, "@docs the sale ends tonight: take the banner down tomorrow morning.", { mentions: [DOCS_BOT] }),
    msg(11, SPRING_SALE, { kind: "bot", id: DOCS_BOT, name: "Docs" }, 60 * 24 * 40, "Done: the banner is gone and the sale page redirects to the catalog."),
    msg(20, TRIAGE, maintainer, 60 * 26, "Sorted the 6 issues opened since Monday: 4 labeled, 2 duplicates closed as proposals for you."),
    msg(21, TRIAGE, maintainer, 60 * 5, "@owner #131 asks for PayPal in the checkout. Do we want it on the roadmap, or should I close it as out of scope?", { mentions: ["owner"], ask_owner: true }),
    msg(22, TRIAGE, maintainer, 60 * 4, "@owner #133 reports a crash on Safari 17 that I cannot reproduce. Ask the reporter for a recording?", { mentions: ["owner"] }),
    msg(31, DM_MAINTAINER_RELEASE, release, 52, "Is #121 safe to include? CI was red yesterday.", { mentions: [] }),
    msg(32, DM_MAINTAINER_RELEASE, maintainer, 47, "Red was a flaky test, fixed in #120. Safe.", { reply_to: 31, hops: 2 }),
    msg(33, DM_MAINTAINER_RELEASE, owner, 25, "Wait for my merge of #121 first."),
    msg(34, DM_MAINTAINER_RELEASE, release, 21, "Thanks, tagging at 10:00 once #121 is merged."),
    msg(40, RELEASE_TRAIN, owner, 60 * 3, "@release cut 2.4 this morning, without #118.", { mentions: [RELEASE_BOT] }),
    msg(41, RELEASE_TRAIN, release, 58, "Branch release-2.4 is cut. @maintainer can you check that #118 is not in it?", { mentions: [BOT] }),
    msg(42, RELEASE_TRAIN, maintainer, 55, "Confirmed: #118 is not in 2.4. #119 is, and its checks pass.", { reply_to: 41, hops: 2 }),
    msg(43, RELEASE_TRAIN, release, 54, "@docs please draft the changelog for 2.4.", { mentions: [DOCS_BOT], hops: 3 }),
    msg(44, RELEASE_TRAIN, release, 53, "@docs mention the saved carts too: they are the headline.", { mentions: [DOCS_BOT], hops: 3 }),
    msg(45, RELEASE_TRAIN, maintainer, 9, "@release #119 needs a rebase on release-2.4 before the tag. I can rebase it now.", { mentions: [RELEASE_BOT], hops: 4, reply_to: 42 }),
    msg(46, RELEASE_TRAIN, release, 7, "@owner the tag of 2.4 waits for #119. Should the Maintainer rebase it now, or do you want to look first? @maintainer please wait for the answer.", { mentions: ["owner", BOT], hops: 5, ask_owner: true, reply_to: 45 }),
  ];
  let n = 0;
  const d = (message_id: number, _channel: string, bot_id: string, state: ChannelDelivery["state"], minutesAgo: number, extra: Partial<ChannelDelivery> = {}): ChannelDelivery => ({
    id: ++n, message_id, bot_id, state, reason: null, session_id: null, joined: null, at: clock.ago(minutesAgo), kind: "message", first_message: null, count: null, ...extra,
  });
  const channelDeliveries: ChannelDelivery[] = [
    d(31, DM_MAINTAINER_RELEASE, BOT, "sent", 52, { session_id: "demo-s-bot-maintainer" }),
    d(32, DM_MAINTAINER_RELEASE, RELEASE_BOT, "sent", 47),
    d(33, DM_MAINTAINER_RELEASE, BOT, "sent", 25, { session_id: "demo-s-bot-maintainer" }),
    d(33, DM_MAINTAINER_RELEASE, RELEASE_BOT, "sent", 25),
    d(34, DM_MAINTAINER_RELEASE, BOT, "sent", 21, { session_id: "demo-s-bot-maintainer" }),
    d(40, RELEASE_TRAIN, RELEASE_BOT, "sent", 60 * 3),
    // Docs reads #release-train once a day: this morning's digest listed your message.
    d(40, RELEASE_TRAIN, DOCS_BOT, "sent", 60 * 3 - 20, { kind: "digest", first_message: 40, count: 1 }),
    d(41, RELEASE_TRAIN, BOT, "sent", 58, { session_id: "demo-s-bot-maintainer" }),
    d(42, RELEASE_TRAIN, RELEASE_BOT, "sent", 55),
  ];
  // Docs works on another task: the changelog request waits in its queue, and the next message joins it.
  const first = d(43, RELEASE_TRAIN, DOCS_BOT, "queued", 54, { reason: "waits for the end of the bot's turn" });
  channelDeliveries.push(first, d(44, RELEASE_TRAIN, DOCS_BOT, "combined", 53, { joined: first.id }));
  channelDeliveries.push(d(45, RELEASE_TRAIN, RELEASE_BOT, "skipped", 9, { reason: "hop limit: hop 4, the channel allows 3" }));
  channelDeliveries.push(d(46, RELEASE_TRAIN, BOT, "skipped", 7, { reason: "hop limit: hop 5, the channel allows 3" }));
  return { botChannels, channelMessages, channelDeliveries };
}
