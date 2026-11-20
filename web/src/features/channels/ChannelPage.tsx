// A bot channel or DM (one page for both, in the Chats frame): the header (name, state, purpose and members, Settings
// and × back to Chats, archiving nothing), the messages (newest at
// the bottom, opened tail-first, older pages as you scroll up), each with its deliveries as muted lines (who woke up
// and why, who is busy, who was skipped and why), the loop-guard footer once bots answer each other (Allow 10 more
// when a limit stopped a delivery) and your message box with @ suggestions of the member bots. Only a bot that asks
// you (or mentions you) adds a Today row; the page marks the channel read once a used, focused window showed it. Its
// settings (`ChannelSettingsPage`): name and purpose, members, Pause, Block (DMs), Archive and Delete.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate, useParams, useSearch } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Fragment, memo, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { ActionButton } from "@/components/ActionButton";
import { BotAvatar, OwnerAvatar, botInk } from "@/components/BotAvatar";
import { ChatMessage } from "@/features/chat/Message";
import { Icon } from "@/components/Icon";
import { ImageThumb } from "@/components/ImageViewer";
import { Markdown } from "@/components/Markdown";
import { toastError } from "@/components/Toast";
import { Badge, Empty, ErrorState, IconButton, Loading, PageHeader } from "@/components/ui";
import { ChannelComposer } from "./ChannelComposer";
import { ChannelMembers } from "./ChannelMembers";
import { useWindowFocused } from "@/features/agents/useSeen";
import type { Channel } from "@/gen/Channel";
import type { ChannelFile } from "@/gen/ChannelFile";
import type { ChannelMessage } from "@/gen/ChannelMessage";
import type { ChannelPage as Page } from "@/gen/ChannelPage";
import type { ScheduledPost } from "@/gen/ScheduledPost";
import { runAction } from "@/lib/actions";
import { useUserActive } from "@/lib/activity";
import { api } from "@/lib/api";
import { attachmentSrc } from "@/lib/attachments";
import { daemonAsset } from "@/lib/connection";
import { deliveryLines, loopFooter, loopSkip, type DeliveryLine } from "@/lib/channels";
import { ago, time, when } from "@/lib/format";
import { usePhone } from "@/lib/phone";
import { PhoneBar } from "@/mobile/PhoneHeader";
import { phoneBack } from "@/mobile/back";

export function ChannelPage() {
  const { id } = useParams({ strict: false }) as { id: string };
  // One page per channel: nothing of the previous one (older pages, a draft) stays.
  return <ChannelPageOf key={id} id={id} />;
}

/** Older pages loaded by scrolling up, merged under the newest page (which the bus refreshes). */
type Older = { messages: ChannelMessage[]; deliveries: Page["deliveries"]; more: boolean | null };

/** The channel's state in words, when it is not the usual one. */
const stateOf = (c: Channel) => c.archived ? "Archived" : c.blocked ? "Blocked: its bots cannot post" : c.paused ? "Paused: messages post, nobody wakes up" : null;

function ChannelPageOf({ id }: { id: string }) {
  const phone = usePhone();
  const qc = useQueryClient();
  const navigate = useNavigate();
  const page = useQuery({ queryKey: ["channel", id], queryFn: () => api.get<Page>(`/api/channels/${id}`), retry: false });
  const [older, setOlder] = useState<Older>({ messages: [], deliveries: [], more: null });
  const channel = page.data?.channel;
  // Older links opened the Members panel (`?panel=members`): members are in the channel's settings now.
  const panel = (useSearch({ strict: false }) as { panel?: string }).panel;
  useEffect(() => { if (panel) void navigate({ to: "/channels/$id/settings", params: { id }, replace: true }); }, [panel, id, navigate]);
  const { messages, deliveries, more } = useMemo(() => {
    const p = page.data;
    if (!p) return { messages: [], deliveries: [], more: false };
    const msgs = new Map([...older.messages, ...p.messages].map((m) => [m.id, m]));
    const dels = new Map([...older.deliveries, ...p.deliveries].map((d) => [d.id, d]));
    return { messages: [...msgs.values()].sort((a, b) => a.id - b.id), deliveries: [...dels.values()].sort((a, b) => a.id - b.id), more: older.more ?? p.more };
  }, [page.data, older]);
  const names = useMemo(() => new Map((channel?.members ?? []).map((m) => [m.bot_id, m.name])), [channel?.members]);
  const lines = useMemo(() => deliveryLines(channel?.kind ?? "channel", messages, deliveries, (b) => names.get(b) ?? "A bot"), [channel?.kind, messages, deliveries, names]);
  const byId = useMemo(() => new Map(messages.map((m) => [m.id, m])), [messages]);
  const footer = channel ? loopFooter(channel, messages.at(-1), deliveries) : null;

  // Read once a focused window someone uses showed it for a moment, like a session's seen mark.
  const focused = useWindowFocused();
  const active = useUserActive();
  const last = channel?.last_message ?? null;
  const due = !!channel && focused && active && last != null && channel.owner_read < last;
  useEffect(() => {
    if (!due || last == null) return;
    const t = setTimeout(() => void runAction("channel.read", { id }, { through: last }).then(() => qc.invalidateQueries({ queryKey: ["channels"] })).catch(() => {}), 1500);
    return () => clearTimeout(t);
  }, [due, id, last, qc]);

  const [loadingMore, setLoadingMore] = useState(false);
  const loadOlder = async () => {
    const first = messages[0];
    if (!first || loadingMore || !more) return;
    setLoadingMore(true);
    try {
      const p = await api.get<Page>(`/api/channels/${id}?before=${first.id}`);
      setOlder((o) => ({ messages: [...p.messages, ...o.messages], deliveries: [...p.deliveries, ...o.deliveries], more: p.more }));
    } catch (e) { toastError(e); } finally { setLoadingMore(false); }
  };

  if (page.isLoading) return <Loading what="the channel" />;
  if (page.error || !channel) return <div className="flex h-full flex-col"><PageHeader title="Channel" /><ErrorState error={page.error ?? new Error("This channel no longer exists.")} onRetry={() => void page.refetch()} action={<Link to="/chats" className="btn btn-sm">All chats</Link>} /></div>;

  const state = stateOf(channel);
  const refresh = () => Promise.all([qc.invalidateQueries({ queryKey: ["channel", id] }), qc.invalidateQueries({ queryKey: ["channels"] })]);
  // Its bots, each opening its settings, then you.
  const members = <span className="flex min-w-0 flex-wrap items-center gap-x-1 gap-y-0.5" data-tour="channel.meta" aria-label={`Members: ${[...channel.members.map((m) => m.name), "you"].join(", ")}`}>
    {channel.members.map((m) => <Fragment key={m.bot_id}>
      <Link to="/bots/$id/settings" params={{ id: m.bot_id }} className="inline-flex min-h-6 min-w-0 items-center gap-1 rounded-control px-0.5 text-ink-2 no-underline hover:bg-sunk hover:text-ink" title={`Settings of ${m.name}`}><BotAvatar name={m.name} size={16} /><span className="truncate">{m.name}</span></Link>
      <span className="text-ink-3" aria-hidden>·</span>
    </Fragment>)}
    <span>you</span>
  </span>;
  const settings = <Link to="/channels/$id/settings" params={{ id }} className="btn btn-sm no-underline" data-tour="channel.settings" title={channel.kind === "dm" ? "Pause or block this DM" : "Members, name and purpose, pause, archive and delete"}><Icon name="gear" size={12} />Settings</Link>;
  const header = phone
    ? <PhoneBar title={<span className="grid min-w-0"><span className="truncate">{channel.title}</span><span className="truncate text-[12px] font-normal text-ink-3">{state ?? `${[...channel.members.map((m) => m.name), "you"].join(", ")}`}</span></span>} onBack={() => phoneBack("/chats")} backLabel="All chats"
      end={<IconButton icon="gear" label={`Settings of ${channel.title}`} data-tour="channel.settings" onClick={() => void navigate({ to: "/channels/$id/settings", params: { id } })} />} />
    : <PageHeader title={channel.title} meta={<>{state && <span className="text-attn">{state}</span>}{channel.kind === "channel" && channel.purpose && <span className="min-w-0 truncate">{channel.purpose}</span>}{members}</>}
      actions={<>{settings}<IconButton icon="x" label="Close the channel" onClick={() => void navigate({ to: "/chats", search: {} as never })} /></>} />;
  return (
    <div className="flex h-full min-h-0 flex-col" data-tour="channel.page">
      {header}
      <div className="flex min-h-0 flex-1 flex-col">
        <MessageList channel={channel} messages={messages} lines={lines} byId={byId} more={more} loadingMore={loadingMore} onOlder={() => void loadOlder()} />
        {footer && <div className={`flex flex-wrap items-center gap-2 border-t border-line-2 px-5 py-1.5 text-[12px] ${footer.stopped ? "bg-attn-soft text-attn" : "text-ink-3"}`} data-tour="channel.guard" title="The loop guard: a bot's message counts one hop more than the message that woke it; your own messages reset the hop and the hourly count">
          <Icon name={footer.stopped ? "alert" : "refresh"} size={12} className="flex-none" />
          <span className="min-w-0 flex-1">{footer.stopped ? `${footer.stopped} · ` : ""}{footer.text}</span>
          {footer.stopped && <ActionButton name="channel.allow-more" params={{ id }} onDone={refresh} />}
        </div>}
        {page.data!.scheduled.length > 0 && <ScheduledPosts posts={page.data!.scheduled} names={names} onDone={refresh} />}
        <ChannelComposer key={channel.id} channel={channel} onSent={refresh} />
      </div>
    </div>
  );
}

/** The bots' scheduled posts that wait (`launchpad channel post|dm --in|--at`), soonest first, each with Cancel. */
function ScheduledPosts({ posts, names, onDone }: { posts: ScheduledPost[]; names: Map<string, string>; onDone: () => unknown }) {
  return <div className="grid max-h-40 gap-1 overflow-y-auto border-t border-line-2 px-5 py-1.5" data-tour="channel.scheduled">
    {posts.map((p) => <div key={p.n} className="flex w-full max-w-text items-center gap-2 rounded-card border border-dashed border-line px-2.5 py-1.5 text-[13px] text-ink-3">
      <Icon name="clock" size={12} className="flex-none" />
      <span className="min-w-0 flex-1">
        <span className="line-clamp-2 break-words whitespace-pre-wrap">{p.text || `${p.files} file${p.files === 1 ? "" : "s"}`}</span>
        <span className="block text-[11px]">{names.get(p.bot_id) ?? "A bot"} scheduled it {ago(p.created_at)} · posts {when(p.due_at)}{p.reply_to != null ? ` · reply to message ${p.reply_to}` : ""}</span>
      </span>
      <ActionButton name="channel.cancel-scheduled" params={{ id: p.channel_id, n: String(p.n) }} icon={null} onDone={onDone} />
    </div>)}
  </div>;
}

// ---------------------------------------------------------------- settings

/** A channel's or DM's settings, on a page of the Chats frame: its name and purpose, its members (named channels),
 * Pause, Block (DMs), Archive or Bring back, and Delete. */
export function ChannelSettingsPage() {
  const { id } = useParams({ strict: false }) as { id: string };
  return <ChannelSettingsOf key={id} id={id} />;
}

function ChannelSettingsOf({ id }: { id: string }) {
  const phone = usePhone();
  const qc = useQueryClient();
  const navigate = useNavigate();
  const page = useQuery({ queryKey: ["channel", id], queryFn: () => api.get<Page>(`/api/channels/${id}`), retry: false });
  const channel = page.data?.channel;
  if (page.isLoading) return <Loading what="the channel" />;
  if (page.error || !channel) return <div className="flex h-full flex-col"><PageHeader title="Settings" /><ErrorState error={page.error ?? new Error("This channel no longer exists.")} onRetry={() => void page.refetch()} action={<Link to="/chats" className="btn btn-sm">All chats</Link>} /></div>;
  const refresh = () => Promise.all([qc.invalidateQueries({ queryKey: ["channel", id] }), qc.invalidateQueries({ queryKey: ["channels"] })]);
  const named = channel.kind === "channel";
  const state = stateOf(channel);
  const open = <Link to="/channels/$id" params={{ id }} className={`btn btn-sm no-underline ${phone ? "" : "btn-primary"}`}><Icon name="comment" size={12} />Open {named ? "the channel" : "the DM"}</Link>;
  const card = "grid min-w-0 grid-cols-[minmax(0,1fr)] gap-2 rounded-card border border-line bg-paper p-4";
  const head = (title: string, action?: ReactNode) => <div className="flex min-h-7 items-center gap-2"><h2 className="seclabel m-0 min-w-0 flex-1">{title}</h2>{action}</div>;
  return <div className="flex h-full min-h-0 flex-col" data-tour="channel.settings-page">
    {phone
      ? <PhoneBar title={`${channel.title} · Settings`} onBack={() => phoneBack(`/channels/${id}`)} backLabel="Back" />
      : <PageHeader title={channel.title} meta={state ?? (named ? channel.purpose || "No purpose yet" : "A DM between two bots")} actions={<>{open}<IconButton icon="x" label="Close the settings" onClick={() => void navigate({ to: "/chats", search: {} as never })} /></>} />}
    <div className="min-h-0 flex-1 overflow-x-hidden overflow-y-auto">
      <div className={`@container grid w-full max-w-page grid-cols-[minmax(0,1fr)] gap-4 py-4 ${phone ? "px-3" : "px-5"}`}>
        {phone && <div className="flex items-center gap-2"><span className="mr-auto min-w-0 truncate text-[12px] text-ink-3">{state ?? (named ? channel.purpose : "A DM between two bots")}</span>{open}</div>}
        <div className="grid grid-cols-[minmax(0,1fr)] gap-4 @5xl:grid-cols-2 @5xl:items-start">
          <div className="grid min-w-0 grid-cols-[minmax(0,1fr)] gap-4">
            {named && <section className={card} data-tour="channel.about">{head("Name and purpose")}<RenameForm channel={channel} onDone={refresh} /></section>}
            {named ? <section className={card} data-tour="channel.members-panel">{head(`Members · ${channel.members.length + 1}`)}<div className="-mx-3"><ChannelMembers channel={channel} /></div></section>
              : <section className={card}>{head("Bots")}<p className="m-0 text-[13px] text-ink-2">{channel.members.map((m) => m.name).join(" and ")} write here, and you. Each is woken by every message of the other.</p></section>}
          </div>
          <div className="grid min-w-0 grid-cols-[minmax(0,1fr)] gap-4">
            <section className={card} data-tour="channel.pause">{head("Deliveries")}
              <p className="m-0 text-[12px] text-ink-3">{channel.paused ? "Paused: messages still post, nobody wakes up." : "Bots are woken by the messages their mode asks for."}</p>
              <span className="w-fit"><ActionButton name="channel.pause" params={{ id }} body={{ paused: !channel.paused }} icon={channel.paused ? "play" : "stop"} title={channel.paused ? "Deliveries start again" : "Stop every delivery in this channel: messages still post, nobody wakes up"} onDone={refresh}>{channel.paused ? "Resume" : "Pause"}</ActionButton></span>
              {!named && <span className="w-fit"><ActionButton name="channel.block" params={{ id }} body={{ blocked: !channel.blocked }} icon={channel.blocked ? "play" : "x"} title={channel.blocked ? "Its bots can post again" : "Its bots can no longer post; you still can"} onDone={refresh}>{channel.blocked ? "Unblock this DM" : "Block this DM"}</ActionButton></span>}
            </section>
            {named && <section className={card} data-tour="channel.archive">{head(channel.archived ? "Archived" : "Archive and delete")}
              <p className="m-0 text-[12px] text-ink-3">{channel.archived ? "Nobody posts here; its history stays. Bring it back to post again." : "Archive: nobody posts, its history stays under Archived in Chats. Delete: its messages and deliveries go for good."}</p>
              <div className="flex flex-wrap items-center gap-2">
                <ActionButton name="channel.archive" params={{ id }} body={{ archived: !channel.archived }} icon={channel.archived ? "refresh" : "archive"} title={channel.archived ? "Its bots can post again" : "Nobody posts; its history stays"} onDone={refresh}>{channel.archived ? "Bring back" : "Archive"}</ActionButton>
                <ActionButton name="channel.delete" params={{ id }} vars={{ name: channel.title }} onDone={async () => { await qc.invalidateQueries({ queryKey: ["channels"] }); void navigate({ to: "/chats", search: {} as never }); }} />
              </div>
            </section>}
          </div>
        </div>
      </div>
    </div>
  </div>;
}

// ---------------------------------------------------------------- messages

function MessageList({ channel, messages, lines, byId, more, loadingMore, onOlder }: { channel: Channel; messages: ChannelMessage[]; lines: Map<number, DeliveryLine[]>; byId: Map<number, ChannelMessage>; more: boolean; loadingMore: boolean; onOlder: () => void }) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const atBottom = useRef(true);
  const v = useVirtualizer({ count: messages.length, getItemKey: (i) => messages[i].id, getScrollElement: () => scrollRef.current, estimateSize: () => 72, overscan: 6, useFlushSync: false });
  const total = v.getTotalSize();
  // Follow the newest message while at the bottom. The real bottom, not the last item's offset: "Load earlier
  // messages" sits above the virtual rows, so item offsets are shorter than the scroll height.
  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (el && atBottom.current && messages.length) el.scrollTop = el.scrollHeight;
  }, [messages.length, total]);
  // Loading above keeps the message that was on top in view.
  const anchor = useRef<{ id: number; index: number } | null>(null);
  useLayoutEffect(() => {
    const a = anchor.current;
    if (!a) return;
    const i = messages.findIndex((m) => m.id === a.id);
    if (i < 0) anchor.current = null;
    else if (i !== a.index) { v.scrollToIndex(i, { align: "start" }); anchor.current = null; }
  }, [messages, v]);
  const older = () => {
    if (!more || loadingMore) return;
    const top = scrollRef.current?.scrollTop ?? 0;
    const shown = v.getVirtualItems().find((it) => it.end > top);
    anchor.current = shown ? { id: Number(shown.key), index: shown.index } : null;
    atBottom.current = false;
    onOlder();
  };
  // Scrolling up loads older messages. A wheel or swipe at the very top fires no scroll event, so check again
  // once a load ended: still near the top (or the messages do not fill the view) loads the next page.
  const olderRef = useRef(older);
  olderRef.current = older;
  useEffect(() => {
    if (!more || loadingMore) return;
    const frame = requestAnimationFrame(() => { if ((scrollRef.current?.scrollTop ?? Infinity) < 150) olderRef.current(); });
    return () => cancelAnimationFrame(frame);
  }, [more, loadingMore, messages.length]);
  return <div ref={scrollRef} className="lp-quiet-scroll min-h-0 flex-1 overflow-x-hidden overflow-y-auto px-5 [overflow-wrap:anywhere]" data-tour="channel.messages"
    onScroll={(e) => {
      const el = e.currentTarget;
      atBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < 60;
      if (atBottom.current) anchor.current = null;
      if (el.scrollTop < 150) older();
    }}>
    <div className="w-full max-w-text">
      {messages.length === 0 ? <Empty icon="comment">{channel.kind === "dm" ? "No message yet. Both bots see what you write here." : "No message yet. Mention a bot with @ to wake it."}</Empty> : <>
        {more && <div className="flex justify-center pt-2"><button className="btn btn-sm" disabled={loadingMore} onClick={older}>{loadingMore ? "Loading…" : "Load earlier messages"}</button></div>}
        <div style={{ height: total, position: "relative" }}>
          {v.getVirtualItems().map((it) => {
            const m = messages[it.index];
            // A reply to the message just above needs no quote.
            const quoted = m.reply_to != null && m.reply_to !== messages[it.index - 1]?.id;
            const reply = quoted ? byId.get(m.reply_to!) ?? null : null;
            return <div key={it.key} data-index={it.index} ref={v.measureElement} className="absolute top-0 right-0 left-0" style={{ transform: `translateY(${it.start}px)` }}>
              <MessageRow m={m} lines={lines.get(m.id)} reply={reply} quoted={quoted} />
            </div>;
          })}
        </div>
      </>}
      <div className="h-3" />
    </div>
  </div>;
}

const firstLine = (t: string) => t.trim().split("\n").find((l) => l.trim()) ?? "";
const NAME_LINK = "text-inherit no-underline hover:underline";

/** One message (`ChatMessage`, like the bot chat): its author's mark and name, the time, what it answers, its text
 * and files in the bubble, its deliveries under it. */
const MessageRow = memo(function MessageRow({ m, lines, reply, quoted }: { m: ChannelMessage; lines: DeliveryLine[] | undefined; reply: ChannelMessage | null; quoted: boolean }) {
  const owner = m.author.kind === "owner";
  const mentionsYou = !owner && m.mentions.includes("owner");
  const today = new Date(m.created_at).toDateString() === new Date().toDateString();
  const name = owner ? "You" : m.author.name;
  return <ChatMessage tour="channel.message" look={owner ? "owner" : "bot"} avatar={owner ? <OwnerAvatar /> : <BotAvatar name={name} />}
    name={<span className={owner ? "text-ink" : botInk(name)}>{m.author.kind === "bot" ? <Link to="/bots/$id" params={{ id: m.author.id }} className={NAME_LINK}>{name}</Link> : name}</span>}
    tag={m.ask_owner ? <Badge tone="attn">Asks you</Badge> : mentionsYou ? <Badge tone="attn">Mentions you</Badge> : null}
    time={<span title={`${when(m.created_at)} · message ${m.id} · hop ${m.hops}`}>{today ? time(m.created_at) : ago(m.created_at)}</span>}
    after={!!lines?.length && <div className="grid w-full min-w-0 gap-0.5">{lines.map((l, i) => <Delivery key={i} l={l} channel={m.channel_id} />)}</div>}>
    {quoted && <div className="mb-1 truncate border-l-2 border-line pl-2 text-[12px] text-ink-3" title={reply ? reply.text : undefined}>↳ Reply to {reply ? `${reply.author.kind === "owner" ? "you" : reply.author.name}: ${firstLine(reply.text)}` : `message ${m.reply_to}`}</div>}
    {m.text.trim() ? <Markdown text={m.text} className="text-[13px]" /> : null}
    {m.files.length > 0 && <MessageFiles m={m} />}
  </ChatMessage>;
}, (a, b) => a.m === b.m && a.reply === b.reply && a.quoted === b.quoted && JSON.stringify(a.lines) === JSON.stringify(b.lines));

const SHOWN_IMAGES = ["image/png", "image/jpeg", "image/webp", "image/gif"];

/** "820 B", "14 KB", "2.3 MB". */
function fileSize(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${Math.round(n / 1024)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

/** A message's files: images as thumbnails (a click opens them), every file as a download link with its size. */
function MessageFiles({ m }: { m: ChannelMessage }) {
  const href = (i: number) => daemonAsset(`/api/channels/${encodeURIComponent(m.channel_id)}/messages/${m.id}/files/${i}`);
  return <div className="flex flex-wrap items-start gap-2 pt-1" data-tour="channel.files">
    {m.files.map((f: ChannelFile, i) => (
      <div key={i} className="grid max-w-full gap-1 rounded-control border border-line bg-chrome p-1.5">
        {SHOWN_IMAGES.includes(f.mime) && <ImageThumb src={attachmentSrc(f.reference)} name={f.name} className="h-24 max-w-48 object-contain" />}
        <a href={href(i)} download={f.name} className="flex min-w-0 items-center gap-1.5 text-[12px] text-ink no-underline hover:underline" title={`Download ${f.name}`}>
          <Icon name="file" size={12} className="flex-none text-ink-3" />
          <span className="max-w-48 truncate">{f.name}</span>
          <span className="flex-none text-ink-3">{fileSize(f.size)}</span>
        </a>
      </div>
    ))}
  </div>;
}

/** Why a delivery waits, from its row's reason (`hooks::QUEUED_*` on Home): "Docs is busy · message queued (1)". */
function queuedLine(bot: string, reason: string | null | undefined, what: string): string {
  if (reason?.startsWith("waits for confirmed quota")) return `${bot} waits for confirmed quota · ${what}`;
  if (reason?.startsWith("waits behind")) return `${bot} has earlier messages waiting · ${what}`;
  if (reason?.startsWith("waits for a free slot")) return `${bot} works in as many chats as it may · ${what}`;
  return `${bot} is busy · ${what}`;
}

/** A delivery as one muted line: "Maintainer woke up (mention) · open its chat", "Docs is busy · message queued (1)". */
function Delivery({ l, channel }: { l: DeliveryLine; channel: string }) {
  // The bot's thread of this channel (`?thread=` takes the thread's key).
  const chat = <Link to="/bots/$id" params={{ id: l.botId }} search={{ thread: `channel:${channel}` } as never} className="text-teal-ink no-underline hover:underline" title="Its chat for this channel, where it answers and works">open its chat</Link>;
  const row = "flex min-w-0 items-start gap-1.5 text-[12px]";
  switch (l.kind) {
    case "woke":
      return <div className={`${row} text-ink-3`} data-tour="channel.delivery"><Icon name="play" size={12} className="mt-0.5 flex-none" /><span className="min-w-0 break-words">{l.bot} woke up ({l.why}) · {chat}</span></div>;
    case "queued":
      return <div className={`${row} text-ink-3`} title={l.reason ?? undefined}><Icon name="clock" size={12} className="mt-0.5 flex-none" /><span className="min-w-0 break-words">{queuedLine(l.bot, l.reason, `message queued (${l.waiting})`)}</span></div>;
    case "combined":
      return <div className={`${row} text-ink-3`}><Icon name="collapse" size={12} className="mt-0.5 flex-none" /><span className="min-w-0 break-words">{l.bot}: combined with {l.with != null ? `message ${l.with}` : "an earlier message"}{l.session ? <> · {chat}</> : null}</span></div>;
    case "digest": {
      const n = `${l.count} message${l.count === 1 ? "" : "s"}`;
      if (l.state === "skipped") return <div className={`${row} ${loopSkip(l.reason) ? "text-attn" : "text-ink-3"}`} title={l.reason ?? undefined}><Icon name="x" size={12} className="mt-0.5 flex-none" /><span className="min-w-0 break-words">{l.bot} · daily digest ({n}) skipped: {l.reason ?? "no reason recorded"}</span></div>;
      if (l.state === "queued") return <div className={`${row} text-ink-3`} title={l.reason ?? undefined}><Icon name="clock" size={12} className="mt-0.5 flex-none" /><span className="min-w-0 break-words">{queuedLine(l.bot, l.reason, `daily digest (${n}) queued`)}</span></div>;
      return <div className={`${row} text-ink-3`} data-tour="channel.digest"><Icon name="book" size={12} className="mt-0.5 flex-none" /><span className="min-w-0 break-words">{l.bot} got the daily digest ({n}) · {chat}</span></div>;
    }
    case "skipped":
      return <div className={`${row} ${l.loop ? "text-attn" : "text-ink-3"}`} title={l.reason}><Icon name={l.loop ? "alert" : "x"} size={12} className="mt-0.5 flex-none" /><span className="min-w-0 break-words">{l.bot} skipped: {l.reason}</span></div>;
  }
}

// ---------------------------------------------------------------- rename

function RenameForm({ channel, onDone }: { channel: Channel; onDone: () => unknown }) {
  const [name, setName] = useState(channel.name);
  const [purpose, setPurpose] = useState(channel.purpose);
  const [busy, setBusy] = useState(false);
  const dirty = name.trim() !== channel.name || purpose.trim() !== channel.purpose;
  const save = async () => {
    setBusy(true);
    try { await runAction("channel.rename", { id: channel.id }, { name: name.trim(), purpose: purpose.trim() }); await onDone(); } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  return <div className="grid gap-2">
    <label className="grid gap-1 text-[12px] font-semibold text-ink-2">Name<input className="field-input font-normal" value={name} maxLength={40} onChange={(e) => setName(e.target.value)} /></label>
    <label className="grid gap-1 text-[12px] font-semibold text-ink-2">Purpose<input className="field-input font-normal" value={purpose} maxLength={200} onChange={(e) => setPurpose(e.target.value)} /></label>
    {dirty && <div className="flex justify-end gap-2">
      <button className="btn btn-sm btn-ghost" onClick={() => { setName(channel.name); setPurpose(channel.purpose); }}>Discard</button>
      <button className="btn btn-sm btn-primary" disabled={busy || !name.trim()} onClick={() => void save()}>Save</button>
    </div>}
  </div>;
}
