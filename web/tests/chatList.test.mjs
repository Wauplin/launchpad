import assert from "node:assert/strict";
import { test } from "node:test";
import { buildChats, chatSections, chatTitle, filterChats, participantOrder } from "../src/lib/chatList.ts";

const bot = (id, workspace_id = "w") => ({ id, workspace_id, settings: { name: id.toUpperCase() } });
let n = 0;
// A bot session: Done and seen, unless `extra` says otherwise.
const session = (botId, thread, at, extra = {}) => ({ id: `s${++n}`, title: `session ${thread}`, prompt: `ask ${thread}`, status: "waiting", started_at: at, status_since: at, seen_at: at + 1, open_question: null, capacity_retry_at: null, last_message: `answer ${thread}`, launch_spec: { bot: { id: botId, thread } }, ...extra });
const thread = (bot_id, id, created_at, extra = {}) => ({ id, bot_id, key: null, title: `ask ${id}`, created_at, archived_at: null, conversation: `${bot_id}:${id}`, ...extra });
const channel = (id, extra = {}) => ({ id, workspace_id: "w", kind: "channel", name: id, title: `#${id}`, purpose: "", members: [], unread: 0, archived: false, last_at: 50, created_at: 1, last_text: null, last_by: null, ...extra });

test("participants keep the order they first appeared, whatever their names", () => {
  // C called by you, then A, then B.
  const threads = [thread("a", "t2", 20, { conversation: "c:t1" }), thread("c", "t1", 10), thread("b", "t3", 30, { conversation: "c:t1" }), thread("c", "t4", 40, { conversation: "c:t1" })];
  assert.deepEqual(participantOrder(threads, "c"), ["c", "a", "b"]);
});

test("one row per conversation, with its participants, its most urgent state and its sections", () => {
  const bots = [bot("a"), bot("b"), bot("c"), bot("x", "other")];
  const threads = new Map([
    ["c", [thread("c", "t1", 10), thread("c", "done", 5), thread("c", "old", 1, { archived_at: 2 })]],
    ["a", [thread("a", "t2", 20, { conversation: "c:t1" }), thread("a", "q", 3)]],
    ["b", [thread("b", "t3", 30, { conversation: "c:t1" }), thread("b", "unseen", 4)]],
    ["x", [thread("x", "elsewhere", 1)]],
  ]);
  const sessions = [
    session("c", "t1", 100), session("a", "t2", 110, { status: "working" }), session("b", "t3", 120),
    session("c", "done", 90), session("a", "q", 80, { open_question: true }), session("b", "unseen", 70, { seen_at: 0 }),
    session("c", "old", 60, { status: "closed" }), session("x", "elsewhere", 50, { status: "working" }),
  ];
  const chats = buildChats({ bots, sessions, threads, channels: [channel("news", { unread: 2 }), channel("quiet"), channel("gone", { archived: true })], workspaceId: "w" });
  const conv = chats.find((c) => c.id === "c:t1");
  assert.deepEqual(conv.participants.map((b) => b.id), ["c", "a", "b"]);
  assert.equal(conv.bot.id, "c");
  assert.equal(conv.thread, "t1");
  assert.equal(conv.section, "Running");
  assert.equal(conv.sessionIds.length, 3);
  const sections = Object.fromEntries(chatSections(chats).map((g) => [g.section, g.chats.map((c) => c.id)]));
  assert.deepEqual(sections, { "Needs you": ["a:q"], Unread: ["b:unseen", "news"], Running: ["c:t1"], Done: ["c:done"], Channels: ["quiet"], Archived: ["c:old", "gone"] });
  assert.ok(!chats.some((c) => c.id === "x:elsewhere"));
  // The phone's pinned chats come first, out of their sections; an archived pin stays in Archived.
  const pinnedFirst = chatSections(chats, ["channel:quiet", "conversation:c:t1", "conversation:c:old"]).map((g) => [g.section, g.chats.map((c) => c.id)]);
  assert.deepEqual(pinnedFirst[0], ["Pinned", ["quiet", "c:t1"]]);
  assert.ok(!pinnedFirst.some(([s]) => s === "Running" || s === "Channels"));
});

test("a follow-up thread keeps the conversation; routine threads are marked", () => {
  const bots = [bot("a")];
  const threads = new Map([["a", [thread("a", "first", 1, { archived_at: 5 }), thread("a", "next", 6, { conversation: "a:first", title: "ask first" }), thread("a", "run", 7, { key: "routine:daily.toml", title: "Daily check" })]]]);
  const sessions = [session("a", "first", 2, { status: "closed", title: "Plan the release", prompt: "ask first" }), session("a", "next", 8, { prompt: "next question" }), session("a", "run", 9, { launch_spec: { bot: { id: "a", thread: "run" }, from: { kind: "routine", name: "Daily check" } } })];
  const chats = buildChats({ bots, sessions, threads, channels: [], workspaceId: "w" });
  const conv = chats.find((c) => c.id === "a:first");
  assert.equal(conv.thread, "next");
  assert.equal(conv.archived, false);
  assert.equal(chats.find((c) => c.id === "a:run").routine, "Daily check");
  // The continued chat keeps the automatic name of its first session.
  assert.equal(conv.title, "Plan the release");
  assert.equal(conv.routine, null);
});

test("the bot filter needs every selected bot in the chat, members of channels included", () => {
  const bots = [bot("a"), bot("b"), bot("c")];
  const threads = new Map([["a", [thread("a", "t1", 1), thread("a", "solo", 2)]], ["b", [thread("b", "t2", 3, { conversation: "a:t1" })]], ["c", []]]);
  const sessions = [session("a", "t1", 10), session("b", "t2", 11), session("a", "solo", 12)];
  const chats = buildChats({ bots, sessions, threads, workspaceId: "w", channels: [channel("ab", { members: [{ bot_id: "a", name: "A" }, { bot_id: "b", name: "B" }] }), channel("c", { members: [{ bot_id: "c", name: "C" }] })] });
  const ids = (list) => list.map((c) => c.id).sort();
  assert.deepEqual(ids(filterChats(chats, { bots: ["a", "b"] })), ["a:t1", "ab"]);
  assert.deepEqual(ids(filterChats(chats, { bots: ["b"] })), ["a:t1", "ab"]);
  assert.deepEqual(ids(filterChats(chats, { bots: ["a", "c"] })), []);
  assert.deepEqual(ids(filterChats(chats, { needle: "solo" })), ["a:solo"]);
  // A message search hit names one of the conversation's sessions.
  const hit = chats.find((c) => c.id === "a:t1").sessionIds[1];
  assert.deepEqual(ids(filterChats(chats, { needle: "zzz", matched: new Set([hit]) })), ["a:t1"]);
  // A channel whose messages match (`/api/chat-search`).
  assert.deepEqual(ids(filterChats(chats, { needle: "zzz", matched: new Set([hit]), matchedChannels: new Set(["c"]) })), ["a:t1", "c"]);
});

test("titles: an automatic description, also for continued chats and routine runs, unless you renamed it", () => {
  const first = { title: "Fix the flaky upload test", prompt: "can you look at the upload test that fails\nthanks" };
  assert.equal(chatTitle({ title: "can you look at the upload test that fails", key: null }, first), "Fix the flaky upload test");
  assert.equal(chatTitle({ title: "can you look at the upload…", key: null }, first), "Fix the flaky upload test");
  assert.equal(chatTitle({ title: "Upload test", key: null }, first), "Upload test");
  // A chat that continued in a new thread: the new session's prompt is the next message; the first session names it.
  const next = { title: "and the download one too?", prompt: "and the download one too?" };
  assert.equal(chatTitle({ title: "can you look at the upload test that fails", key: null }, first, next), "Fix the flaky upload test");
  // A routine run: its session's automatic name, not the routine's name again.
  const run = { title: "CI is green on main", prompt: "Routine “Daily check”: check CI" };
  assert.equal(chatTitle({ title: "Daily check", key: "routine:daily.toml" }, run), "CI is green on main");
  // Not named yet: the thread's own title.
  assert.equal(chatTitle({ title: "Daily check", key: "routine:daily.toml" }, { title: "Routine “Daily check”: check CI", prompt: "Routine “Daily check”: check CI" }), "Daily check");
  assert.equal(chatTitle(null, null), "New chat");
  // Renamed (by hand or with Auto): the thread's title wins, also for a routine run or a title that starts the prompt.
  assert.equal(chatTitle({ title: "Weekly CI report", key: "routine:daily.toml", renamed: true }, run), "Weekly CI report");
  assert.equal(chatTitle({ title: "can you look at the upload", key: null, renamed: true }, first), "can you look at the upload");
});

test("channels and pinned chats list by name, ignoring case; other sections stay newest first", () => {
  const channels = [channel("beta", { last_at: 90 }), channel("Alpha", { last_at: 10 }), channel("zeta", { last_at: 99 }), channel("10-x"), channel("9-x")];
  const chats = buildChats({ bots: [], sessions: [], threads: new Map(), channels, workspaceId: "w" });
  assert.deepEqual(chatSections(chats)[0].chats.map((c) => c.id), ["9-x", "10-x", "Alpha", "beta", "zeta"]);
  const pinned = chatSections(chats, ["channel:zeta", "channel:beta", "channel:Alpha"]);
  assert.deepEqual(pinned[0].chats.map((c) => c.id), ["Alpha", "beta", "zeta"]);
  assert.deepEqual(chats.map((c) => c.id).slice(0, 2), ["zeta", "beta"]);
});

test("a routine run's handoffs fold under its row and only their questions, failures and work count", () => {
  const bots = [bot("h"), bot("py"), bot("docs")];
  const run = "h:run";
  const threads = new Map([
    ["h", [thread("h", "run", 10, { key: "routine:brief.toml", run })]],
    // Python's #hub thread that the run's post woke, and Docs' thread that Python's post woke in turn.
    ["py", [thread("py", "hub", 20, { key: "channel:hub", run })]],
    ["docs", [thread("docs", "hub", 30, { key: "channel:hub", run }), thread("docs", "own", 5)]],
  ]);
  const build = (extra = {}) => buildChats({ bots, sessions: [session("h", "run", 100), session("py", "hub", 110, { seen_at: 0 }), session("docs", "hub", 120, extra), session("docs", "own", 50)], threads, channels: [], workspaceId: "w" });
  let chats = build();
  const root = chats.find((c) => c.id === run);
  assert.deepEqual(root.children, ["py:hub", "docs:hub"]);
  assert.equal(chats.find((c) => c.id === "py:hub").parent, run);
  // An unseen finished handoff is part of the run: no Unread row of its own, the run stays Done.
  let sections = Object.fromEntries(chatSections(chats).map((g) => [g.section, g.chats.map((c) => c.id)]));
  assert.deepEqual(sections, { Done: [run, "docs:own"] });
  assert.equal(root.sessionIds.length, 3);
  // A handoff that fails or asks surfaces the whole run, naming its state.
  chats = build({ status: "error", seen_at: 0 });
  sections = Object.fromEntries(chatSections(chats).map((g) => [g.section, g.chats.map((c) => c.id)]));
  assert.deepEqual(sections, { Unread: [run], Done: ["docs:own"] });
  assert.equal(chats.find((c) => c.id === run).label, "Failed");
  chats = build({ open_question: true });
  assert.equal(chats.find((c) => c.id === run).section, "Needs you");
  // A run archived without its handoff leaves the handoff a row of its own.
  const archived = new Map([...threads, ["h", [thread("h", "run", 10, { key: "routine:brief.toml", run, archived_at: 200 })]]]);
  chats = buildChats({ bots, sessions: [session("h", "run", 100, { status: "closed" }), session("py", "hub", 110), session("docs", "hub", 120), session("docs", "own", 50)], threads: archived, channels: [], workspaceId: "w" });
  assert.equal(chats.find((c) => c.id === "py:hub").parent, null);
});
