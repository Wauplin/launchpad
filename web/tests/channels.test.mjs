import assert from "node:assert/strict";
import { test } from "node:test";
import { deliveryLines, loopFooter, mentionAt, channelLastLine, channelNameError, archivedOf } from "../src/lib/channelRows.ts";

const msg = (id, extra = {}) => ({ id, channel_id: "c", author: { kind: "bot", id: "r", name: "Release" }, session_id: null, turn: null, text: "hi", mentions: [], reply_to: null, hops: 1, ask_owner: false, created_at: 0, ...extra });
const del = (id, message_id, bot_id, state, extra = {}) => ({ id, message_id, bot_id, state, reason: null, session_id: null, joined: null, at: 0, ...extra });
const names = (b) => ({ m: "Maintainer", d: "Docs", r: "Release" })[b];
const channel = (extra = {}) => ({ kind: "channel", hop_limit: 3, hourly_limit: 30, allowance: 0, hour_count: 5, last_hops: 1, purpose: "", last_by: null, last_text: null, ...extra });

test("deliveries become woke, queued, combined and skipped lines under their message", () => {
  const messages = [msg(1, { mentions: ["m"] }), msg(2), msg(3)];
  const lines = deliveryLines("channel", messages, [
    del(1, 1, "m", "sent", { session_id: "s" }),
    del(2, 1, "r", "sent"),
    del(3, 2, "d", "queued", { reason: "waits" }),
    del(4, 3, "d", "combined", { joined: 3 }),
    del(5, 3, "r", "skipped", { reason: "hop limit: hop 4, the channel allows 3" }),
  ], names);
  assert.deepEqual(lines.get(1).map((l) => [l.kind, l.why]), [["woke", "mention"], ["woke", "subscription"]]);
  assert.equal(lines.get(2)[0].kind, "queued");
  assert.equal(lines.get(2)[0].waiting, 2);
  assert.equal(lines.get(3)[0].with, 2);
  assert.equal(lines.get(3)[1].loop, true);
  assert.equal(deliveryLines("dm", [msg(1)], [del(1, 1, "m", "sent")], names).get(1)[0].why, "DM");
});

test("the loop-guard footer shows once bots answer each other and names the limit that stopped a delivery", () => {
  assert.equal(loopFooter(channel(), msg(1), []), null);
  assert.deepEqual(loopFooter(channel({ last_hops: 2 }), msg(1), []), { text: "Hop 2 of 3 · 5 bot messages this hour (limit 30)", stopped: null });
  const f = loopFooter(channel({ last_hops: 4, hour_count: 1, allowance: 2 }), msg(1), [del(1, 1, "r", "skipped", { reason: "hop limit: hop 4" })]);
  assert.equal(f.stopped, "The hop limit stopped a delivery");
  assert.equal(f.text, "Hop 4 of 3 · 1 bot message this hour (limit 30) · 2 more allowed");
  assert.equal(loopFooter(channel(), msg(1), [del(1, 1, "r", "skipped", { reason: "hourly limit: 31" })]).stopped, "The hourly limit stopped a delivery");
  assert.equal(loopFooter(channel(), msg(1), [del(1, 1, "r", "skipped", { reason: "bot paused" })]), null);
});

test("the @ word before the caret, never inside an address", () => {
  assert.deepEqual(mentionAt("hi @rel", 7), { start: 3, query: "rel" });
  assert.deepEqual(mentionAt("@", 1), { start: 0, query: "" });
  assert.equal(mentionAt("me@example", 10), null);
  assert.equal(mentionAt("@release done", 13), null);
});

test("a row's last line names its author", () => {
  assert.equal(channelLastLine(channel({ last_by: "you", last_text: "Wait" })), "You: Wait");
  assert.equal(channelLastLine(channel({ last_by: "Maintainer", last_text: "Confirmed" })), "Maintainer: Confirmed");
  assert.equal(channelLastLine(channel({ purpose: "Ship it" })), "Ship it");
});

test("a new channel's name is a free slug; archived channels are listed apart", () => {
  assert.equal(channelNameError("release-train", []), null);
  assert.ok(channelNameError("Release", []));
  assert.ok(channelNameError("-x", []));
  assert.ok(channelNameError("a".repeat(41), []));
  assert.match(channelNameError("triage", ["triage"]), /already exists/);
  const all = [{ id: "a", workspace_id: "w", archived: true }, { id: "b", workspace_id: "w", archived: false }, { id: "c", workspace_id: "x", archived: true }];
  assert.deepEqual(archivedOf(all, "w").map((c) => c.id), ["a"]);
});
