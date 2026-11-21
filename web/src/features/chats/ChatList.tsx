// The Chats list (the laptop's side column, the phone's Chats tab): a search and a bot filter over every chat of the
// workspace, grouped by state like Agents (Needs you, Unread, Running, Quota paused, Sleeping, Failed, Done,
// Channels, Archived). Rows look like agent rows: a conversation of bots has its bots' square marks, a channel or DM
// a grey "#" or "↔" mark and the word Channel or DM. A row opens the chat; its ⋯ pins it (a sidebar shortcut),
// renames (in the chat's header), mutes a channel for you (no unread mark or count), archives or brings back a conversation and opens its trace in Agents. Channels are archived from their
// settings, never from this list. On the phone, swiping an open conversation's row to the right uncovers Archive; only a
// tap on it archives (`SwipeReveal`).

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate, useRouterState } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { BotAvatar } from "@/components/BotAvatar";
import { Icon } from "@/components/Icon";
import { MenuItem, Popover } from "@/components/Popover";
import { PullToRefresh } from "@/components/PullToRefresh";
import { SwipeReveal } from "@/components/SwipeReveal";
import { toast, toastError } from "@/components/Toast";
import { ActionMenuItem } from "@/components/ActionButton";
import { Count, Empty, ErrorState, Loading, MoreMenu, RoutineMark, StatusBadge } from "@/components/ui";
import { useArchiveThread, useRenameRequest } from "@/features/chat/BotThreads";
import type { Bot } from "@/gen/Bot";
import { runAction } from "@/lib/actions";
import { api } from "@/lib/api";
import { refreshThreads, useBots } from "@/lib/bots";
import { byName } from "@/lib/channels";
import { chatOf, chatSections, filterChats, type Chat, type ChatSection } from "@/lib/chatList";
import { age } from "@/lib/format";
import { isPhone, usePhone } from "@/lib/phone";
import { isRunning } from "@/lib/sessionStatus";
import { useScope } from "@/lib/workspace";
import { useChatsStore } from "@/stores/chats";
import { useChatPins, useChats, usePinChat } from "./data";
import { ChatMark, participantNames } from "./ChatMark";

/** Row height: the phone's larger text needs more room for the two lines (as in Agents). */
const ROW = isPhone ? 80 : 68;
const HEADER = 34;

/** States whose pill tells more than the section heading. */
const INFORMATIVE = new Set(["Question", "Quota paused", "Failed", "Interrupted", "Retry pending"]);

type RowProps = { chat: Chat; selected: boolean; pinned: boolean; onPin: (c: Chat, pinned: boolean) => void; onArchive: (c: Chat, archived: boolean) => void; onRename: (c: Chat) => void; archiving: boolean };

/** One chat, styled like an agent row. Callbacks are stable, so a bus update re-renders only the rows that changed. */
const ChatRow = memo(function ChatRow({ chat, selected, pinned, onPin, onArchive, onRename, archiving }: RowProps) {
  const navigate = useNavigate();
  const phone = usePhone();
  const thread = chat.kind === "thread" ? chat : null;
  const channel = chat.kind === "channel" ? chat : null;
  const bold = thread ? thread.unseen || thread.tone === "needs" : !!channel && (channel.unread > 0 || channel.asks > 0);
  const who = thread ? participantNames(thread.participants) : channel?.channel.kind === "dm" ? "DM" : "Channel";
  const running = !!thread?.session && isRunning(thread.session);
  const link = thread ? { to: "/bots/$id" as const, params: { id: thread.bot.id }, search: { thread: thread.thread } } : { to: "/channels/$id" as const, params: { id: chat.id }, search: {} };
  const label = `${chat.title}, ${thread ? `chat with ${who}, ${thread.label}` : `${who}${channel?.channel.owner_muted ? ", muted" : ""}${channel?.unread ? `, ${channel.unread} unread` : ""}`}${pinned ? ", pinned" : ""}`;
  return (
    <SwipeReveal enabled={phone && !!thread && !thread.archived}
      action={{ label: "Archive", icon: "archive", disabled: running || archiving, title: running ? "Stop its turn first to archive this chat" : `Archive “${chat.title}”: it stays under Archived`, onClick: () => onArchive(chat, true) }}>
    <div className="group relative" data-tour="chats.row" data-tour-id={chat.id}>
      <Link {...link} aria-label={label} aria-current={selected ? "page" : undefined}
        style={{ height: ROW }} className={`grid w-full grid-cols-[18px_1fr_auto] content-start gap-x-2 gap-y-1 border-b border-line-2 px-3 py-2 text-left text-ink no-underline hover:bg-chrome ${selected ? "bg-select hover:bg-select" : ""}`}>
        <span className="row-span-2 mt-px"><ChatMark chat={chat} /></span>
        <span className={`flex min-w-0 items-center gap-1.5 text-[13px] leading-[1.35] ${chat.archived ? "font-medium text-ink-2" : bold ? "font-semibold" : "font-medium"}`} title={chat.title}>
          {thread?.routine && <RoutineMark routine={thread.routine} />}
          {pinned && <span className="inline-flex shrink-0 text-teal-ink" title="Pinned in the sidebar" aria-hidden><Icon name="pin" size={12} /></span>}
          <span className="min-w-0 truncate">{chat.title}</span>
          {channel?.channel.owner_muted && <span className="inline-flex shrink-0 text-ink-3" title="Muted: new messages add no unread mark" aria-hidden><Icon name="bell-off" size={12} /></span>}
        </span>
        <span className="flex items-center gap-1 text-[12px] whitespace-nowrap text-ink-3">
          {thread && INFORMATIVE.has(thread.label) && !thread.archived ? <StatusBadge tone={thread.tone} label={thread.label} />
            : <>{running && <span className="lp-spin" title="Working" />}{age(chat.last)}{chat.kind === "channel" && !chat.archived && (chat.asks > 0 ? <Count n={chat.asks} needs title={`${chat.asks} message${chat.asks === 1 ? "" : "s"} for you`} /> : chat.unread > 0 ? <span className="size-[7px] rounded-full bg-ink-3" title={`${chat.unread} unread`} /> : null)}</>}
        </span>
        <span className="agent-row-line col-span-2 line-clamp-2 pr-8 text-[12px] leading-[1.3] text-ink-3" title={chat.line ? `${who} · ${chat.line}` : who}>
          {chat.kind === "channel" ? <span className="mr-1.5 inline-flex h-4 items-center rounded border border-line px-1 align-[1px] text-[11px] font-semibold text-ink-2">{who}</span> : <span className="text-ink-2">{who}</span>}
          {chat.line && <>{chat.kind === "thread" ? " · " : ""}<span className={bold ? "text-ink-2" : ""}>{chat.line}</span></>}
        </span>
      </Link>
      <span className="agent-row-action lp-row-more absolute right-1.5 bottom-1.5">
        <MoreMenu label={`More actions for ${chat.title}`}>{(close) => <>
          <MenuItem icon={<Icon name="pin" size={14} />} label={pinned ? "Unpin" : "Pin"} detail={pinned ? "Remove its shortcut from the sidebar" : "A shortcut in the sidebar, first in the phone's list"} onClick={() => { close(); onPin(chat, !pinned); }} />
          {thread && !thread.archived && <MenuItem icon={<Icon name="pencil" size={14} />} label="Rename" detail="Edit its title in the chat's header" onClick={() => { close(); onRename(chat); }} />}
          {thread && (thread.archived
            ? <MenuItem icon={<Icon name="refresh" size={14} />} label="Unarchive" detail="Brings the chat and its sessions back" onClick={() => { close(); onArchive(chat, false); }} />
            : <MenuItem icon={<Icon name="archive" size={14} />} label="Archive" detail={running ? "Stop its turn first" : "It stays under Archived"} disabled={running || archiving} onClick={() => { close(); onArchive(chat, true); }} />)}
          {thread?.session && <MenuItem icon={<Icon name="agents" size={14} />} label="Open in Agents" detail="Its session's trace, terminal and files" onClick={() => { close(); void navigate({ to: "/agents", search: { session: thread.session!.id } as never }); }} />}
          {channel && <ActionMenuItem name="channel.mute" params={{ id: chat.id }} body={{ muted: !channel.channel.owner_muted }} close={close} icon={channel.channel.owner_muted ? "bell" : "bell-off"}
            label={channel.channel.owner_muted ? "Unmute" : "Mute"} detail={channel.channel.owner_muted ? "New messages show as unread again" : "No unread mark or count for you; bots still get every message"} />}
          {chat.kind === "channel" && <MenuItem icon={<Icon name="gear" size={14} />} label="Settings" detail={chat.channel.kind === "dm" ? "Pause or block it" : "Members, name, pause, archive"} onClick={() => { close(); void navigate({ to: "/channels/$id/settings", params: { id: chat.id } }); }} />}
        </>}</MoreMenu>
      </span>
    </div>
    </SwipeReveal>
  );
});

function SectionHeader({ section, count, collapsed, onToggle }: { section: ChatSection; count: number; collapsed: boolean; onToggle: () => void }) {
  const highlight = section === "Needs you" || section === "Unread";
  return <button type="button" aria-expanded={!collapsed} aria-label={`${collapsed ? "Expand" : "Collapse"} ${section}`} data-tour="chats.section" data-tour-id={section} onClick={onToggle}
    className="flex w-full items-center gap-2 px-3 text-left seclabel hover:bg-sunk" style={{ height: HEADER }}>
    <Icon name="chev" size={12} className={collapsed ? "-rotate-90" : ""} />
    {section} <b className={highlight && count ? "text-attn" : "font-normal text-ink-3"}>{count}</b>
  </button>;
}

/** The bot filter: the chats in which every chosen bot takes part (a conversation's bots, a channel's members). */
function BotFilter({ bots }: { bots: Bot[] }) {
  const chosen = useChatsStore((s) => s.bots);
  const setBots = useChatsStore((s) => s.setBots);
  const names = bots.filter((b) => chosen.includes(b.id)).map((b) => b.settings.name);
  const title = names.length ? `Chats with ${names.join(" and ")}` : "Show only the chats of some bots";
  return <Popover viewport width={260} label="Filter by bots" trigger={({ open, toggle }) =>
    <button type="button" className="lp-chip min-w-0" aria-pressed={chosen.length > 0} aria-expanded={open} aria-haspopup="dialog" title={title} data-tour="chats.bot-filter" onClick={toggle}>
      <Icon name="spark" size={12} /><span className="truncate">{chosen.length === 0 ? "Bots" : chosen.length === 1 ? names[0] ?? "1 bot" : `${chosen.length} bots`}</span><Icon name="chev" size={12} />
    </button>}>
    {(close) => <div className="grid gap-px p-1" role="group" aria-label="Bots that must all take part">
      <p className="m-0 px-2 pt-1 pb-1.5 text-[12px] text-ink-3">Chats and channels with every bot you tick.</p>
      {bots.map((b) => <label key={b.id} className="flex min-h-8 cursor-pointer items-center gap-2 rounded-control px-2 text-[13px] hover:bg-sunk">
        <input type="checkbox" checked={chosen.includes(b.id)} onChange={(e) => setBots(e.target.checked ? [...chosen, b.id] : chosen.filter((id) => id !== b.id))} />
        <BotAvatar name={b.settings.name} size={16} /><span className="min-w-0 flex-1 truncate">{b.settings.name}</span>
      </label>)}
      {bots.length === 0 && <p className="m-0 px-2 py-1 text-[12px] text-ink-3">No bot in this workspace.</p>}
      {chosen.length > 0 && <button type="button" className="btn btn-sm btn-ghost mt-1 justify-self-start" onClick={() => { setBots([]); close(); }}>Clear</button>}
    </div>}
  </Popover>;
}

/** What the address shows: a bot's thread or a channel (the selected row). */
function useOpenChat(): { bot: string; thread: string } | { channel: string } | null {
  const loc = useRouterState({ select: (s) => `${s.location.pathname}${s.location.searchStr}` });
  return useMemo(() => {
    const url = new URL(loc, "http://x");
    const bot = /^\/bots\/([^/]+)$/.exec(url.pathname);
    const thread = url.searchParams.get("thread");
    if (bot && thread && thread !== "new") return { bot: bot[1], thread };
    const channel = /^\/channels\/([^/]+)/.exec(url.pathname);
    return channel ? { channel: channel[1] } : null;
  }, [loc]);
}

export function ChatList() {
  const phone = usePhone();
  const scope = useScope();
  const qc = useQueryClient();
  const { chats, loading, error, pinned, archivedMore } = useChats();
  const showMoreArchived = useChatsStore((st) => st.showMoreArchived);
  const allBots = useBots();
  const bots = useMemo(() => byName((allBots.data ?? []).filter((b) => b.workspace_id === scope.workspace.id), (b) => b.settings.name), [allBots.data, scope.workspace.id]);
  const { search, searchOpen, setSearch, bots: chosen, setBots, collapsed, toggle } = useChatsStore();
  const searching = searchOpen || !!search;
  // A bot of another workspace or a deleted one leaves the filter.
  useEffect(() => { if (allBots.data && chosen.some((id) => !bots.some((b) => b.id === id))) setBots(chosen.filter((id) => bots.some((b) => b.id === id))); }, [allBots.data, bots, chosen, setBots]);
  const [debounced, setDebounced] = useState("");
  useEffect(() => { const t = setTimeout(() => setDebounced(search.trim()), 250); return () => clearTimeout(t); }, [search]);
  // Messages too, like Agents' search: the sessions whose messages match.
  const matches = useQuery({ queryKey: ["session-search", debounced], queryFn: () => api.get<string[]>(`/api/sessions/search?q=${encodeURIComponent(debounced)}`), enabled: !!debounced });
  // Channels and DMs search their messages too (`/api/chat-search`).
  const channelMatches = useQuery({ queryKey: ["chat-search", scope.workspace.id, debounced], queryFn: () => api.get<string[]>(`/api/chat-search?workspace=${encodeURIComponent(scope.workspace.id ?? "")}&q=${encodeURIComponent(debounced)}`), enabled: !!debounced && !!scope.workspace.id });
  const fresh = debounced === search.trim();
  const matched = useMemo(() => (fresh && matches.data ? new Set(matches.data) : null), [matches.data, fresh]);
  const matchedChannels = useMemo(() => (fresh && channelMatches.data ? new Set(channelMatches.data) : null), [channelMatches.data, fresh]);
  const shown = useMemo(() => filterChats(chats, { needle: search, bots: chosen, matched, matchedChannels }), [chats, search, chosen, matched, matchedChannels]);
  // The phone has no sidebar: its pinned chats come first.
  const pins = useChatPins().data;
  const pinOrder = useMemo(() => (phone ? (pins ?? []).map((p) => p.target) : undefined), [phone, pins]);
  const groups = useMemo(() => chatSections(shown, pinOrder), [shown, pinOrder]);
  const open = useOpenChat();
  const selected = open ? chatOf(chats, open)?.id ?? null : null;
  const pin = usePinChat();
  const navigate = useNavigate();
  const { archive, busy } = useArchiveThread();

  // Stable callbacks for the memoized rows.
  const actions = useRef({ pin, archive, qc, navigate });
  actions.current = { pin, archive, qc, navigate };
  const onPin = useCallback((c: Chat, on: boolean) => void actions.current.pin(c, on), []);
  // Rename edits the title in place, in the chat's header: open the chat and ask its header for the editor.
  const onRename = useCallback((c: Chat) => {
    if (c.kind !== "thread") return;
    useRenameRequest.setState({ request: { bot: c.bot.id, thread: c.thread } });
    void actions.current.navigate({ to: "/bots/$id", params: { id: c.bot.id }, search: { thread: c.thread } as never });
  }, []);
  const onArchive = useCallback((c: Chat, on: boolean) => {
    if (c.kind !== "thread") return;
    if (on) { void actions.current.archive(c.bot.id, c.thread, c.id); return; }
    void runAction("bot.thread-archive", { id: c.bot.id, thread: c.thread }, { archived: false })
      .then(() => { toast("Chat brought back"); return Promise.all([refreshThreads(actions.current.qc, c.bot.id), actions.current.qc.invalidateQueries({ queryKey: ["sessions"] })]); })
      .catch(toastError);
  }, []);

  type Item = { kind: "section"; section: ChatSection; count: number } | { kind: "chat"; chat: Chat } | { kind: "more"; left: number };
  const items: Item[] = [];
  for (const g of groups) {
    items.push({ kind: "section", section: g.section, count: g.chats.length });
    // A search or a filter shows every match, collapsed sections included.
    if (collapsed[g.section] && !search && !chosen.length) continue;
    for (const chat of g.chats) items.push({ kind: "chat", chat });
    // Home sends a page of archived conversations: Show more loads the next one.
    if (g.section === "Archived" && archivedMore > 0) items.push({ kind: "more", left: archivedMore });
  }
  const listRef = useRef<HTMLDivElement>(null);
  const virtual = useVirtualizer({ count: items.length, getScrollElement: () => listRef.current, estimateSize: (i) => (items[i].kind === "chat" ? ROW : HEADER), getItemKey: (i) => { const it = items[i]; return it.kind === "chat" ? it.chat.id : it.kind === "section" ? `s-${it.section}` : "more"; }, overscan: 6, useFlushSync: false });
  const filtering = !!search.trim() || chosen.length > 0;

  return <div className="flex h-full min-h-0 flex-col bg-paper" data-tour="chats.list">
    <div className="grid gap-2 border-b border-line p-2">
      <div className="flex min-w-0 items-center gap-1.5">
        <button className="lp-chip" aria-pressed={searching} aria-label="Search chats and messages" title={searching ? "Close the search" : "Search chats and messages"} onClick={() => { if (searching) setSearch("", false); else setSearch(search, true); }}><Icon name="search" size={12} /></button>
        <BotFilter bots={bots} />
        {!phone && <Link to="/chats" className="lp-chip ml-auto shrink-0 no-underline" title="Start a chat: every bot and channel" data-tour="chats.start"><Icon name="plus" size={12} />New chat</Link>}
      </div>
      {searching && <input autoFocus={searchOpen} aria-label="Search chats" className="field-input w-full" placeholder="Search chats and messages…" value={search} onChange={(e) => setSearch(e.target.value)} onKeyDown={(e) => { if (e.key === "Escape" && !search) setSearch("", false); }} />}
      {(matches.isFetching || channelMatches.isFetching) && <span className="text-[12px] text-ink-3">Searching messages…</span>}
      {(matches.error ?? channelMatches.error) && <span className="text-[12px] text-danger-ink">{(matches.error ?? channelMatches.error)!.message}</span>}
    </div>
    <div ref={listRef} className="min-h-0 flex-1 overflow-x-hidden overflow-y-auto" aria-label="Chats">
      {phone && <PullToRefresh target={listRef} onRefresh={() => Promise.all([qc.invalidateQueries({ queryKey: ["sessions"] }), refreshThreads(qc), qc.invalidateQueries({ queryKey: ["channels"] })])} />}
      {loading ? <Loading what="chats" /> : error ? <ErrorState error={error} onRetry={() => void allBots.refetch()} />
        : chats.length === 0 ? <div className="grid gap-3 p-4 text-[13px] text-ink-2">{bots.length ? `No chat in ${scope.workspace.name} yet. Start one with a bot.` : `No bot in ${scope.workspace.name} yet.`}
          <span><Link to="/chats" search={(bots.length ? (phone ? { view: "start" } : {}) : { new: true }) as never} className="btn btn-sm">{bots.length ? "Start a chat" : "New bot"}</Link></span></div>
        : shown.length === 0 ? <Empty icon="search" action={<button className="btn btn-sm" onClick={() => { setSearch("", false); setBots([]); }}>Clear search and filter</button>}>{filtering ? "No chat matches." : "No chat."}</Empty>
        : <div style={{ height: virtual.getTotalSize(), position: "relative", flexShrink: 0 }}>
          {virtual.getVirtualItems().map((v) => {
            const item = items[v.index];
            return <div key={v.key} style={{ position: "absolute", top: 0, left: 0, width: "100%", height: v.size, transform: `translateY(${v.start}px)` }}>
              {item.kind === "chat" ? <ChatRow chat={item.chat} selected={item.chat.id === selected} pinned={pinned.has(item.chat.pin)} onPin={onPin} onArchive={onArchive} onRename={onRename} archiving={item.chat.kind === "thread" && busy(item.chat.bot.id, item.chat.thread)} />
                : item.kind === "section" ? <SectionHeader section={item.section} count={item.count} collapsed={!!collapsed[item.section] && !filtering} onToggle={() => toggle(item.section)} />
                : <button className="px-3 py-2 text-left text-[12px] text-teal-ink hover:underline" onClick={showMoreArchived}>Show more ({item.left} more archived)</button>}
            </div>;
          })}
        </div>}
    </div>
  </div>;
}
