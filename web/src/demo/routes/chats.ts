// The Chats list's routes (Home's `crates/api/src/chats.rs`): its threads with a page of archived conversations,
// the channels whose messages match a search, and pinned chats, kept in memory like the rest of the demo.
import { route, publish, DemoError } from "../daemon";
import { world } from "../world";
import type { BotThread } from "@/gen/BotThread";

type Pin = { target: string; workspace_id: string; at: number };
const pins: Pin[] = [];

/** Every thread of the workspace's bots, a closed session counting as archived (Home's `bot_threads::for_chats`). */
function threadsOf(workspace: string | null): BotThread[] {
  const bots = new Set(world.bots.filter((b) => b.workspace_id === workspace).map((b) => b.id));
  return Object.entries(world.botThreads).filter(([bot]) => bots.has(bot)).flatMap(([bot, list]) => list.map((t) => {
    const s = world.sessions.filter((x) => x.launch_spec.bot?.id === bot && (x.launch_spec.bot?.thread || "main") === t.id).sort((a, b) => a.started_at - b.started_at).at(-1);
    return t.archived_at == null && s?.status === "closed" ? { ...t, archived_at: s.status_since } : t;
  }));
}

route("GET", "/api/chat-threads", ({ query }) => {
  const rows = threadsOf(query.get("workspace"));
  const limit = Number(query.get("archived") ?? 30);
  const open = new Set(rows.filter((t) => t.archived_at == null).map((t) => t.conversation));
  const last = new Map<string, number>();
  for (const t of rows) if (!open.has(t.conversation)) last.set(t.conversation, Math.max(last.get(t.conversation) ?? 0, t.archived_at ?? t.created_at));
  const shown = new Set([...last.entries()].sort((a, b) => b[1] - a[1]).slice(0, limit).map(([c]) => c));
  return { threads: rows.filter((t) => open.has(t.conversation) || shown.has(t.conversation)).sort((a, b) => b.created_at - a.created_at), archived_total: last.size };
});

route("GET", "/api/chat-search", ({ query }) => {
  const q = (query.get("q") ?? "").trim().toLowerCase();
  const ws = query.get("workspace");
  if (!q) return [];
  const mine = new Set(world.botChannels.filter((c) => c.workspace_id === ws).map((c) => c.id));
  return [...new Set(world.channelMessages.filter((m) => mine.has(m.channel_id) && m.text.toLowerCase().includes(q)).map((m) => m.channel_id))];
});

route("GET", "/api/chat-pins", ({ query }) => pins.filter((p) => p.workspace_id === query.get("workspace")));
route("POST", "/api/chat-pins", ({ body }) => {
  const target = String(body.target ?? "");
  if (!/^(conversation:[\w-]+:[\w-]+|channel:[\w-]+)$/.test(target)) throw new DemoError(400, "a chat pin names `conversation:<bot>:<thread>` or `channel:<id>`");
  const at = pins.findIndex((p) => p.target === target);
  if (at >= 0) pins.splice(at, 1);
  if (body.pinned) pins.push({ target, workspace_id: String(body.workspace_id), at: Date.now() });
  publish("chats", "pins", { workspace: body.workspace_id });
  return { ok: true, pinned: !!body.pinned };
});
