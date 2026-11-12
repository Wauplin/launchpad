// A bot channel or DM (one page for both): the header (name, purpose and members, Pause, ⋯), the messages (newest at
// the bottom, opened tail-first, older pages as you scroll up), each with its deliveries as muted lines (who woke up
// and why, who is busy, who was skipped and why), the loop-guard footer once bots answer each other (Allow 10 more
// when a limit stopped a delivery) and your message box with @ suggestions of the member bots. Channel messages
// never go into Today here; the page marks the channel read once a used, focused window showed it.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate, useParams } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { ActionButton, ActionMenuItem } from "@/components/ActionButton";
import { BotAvatar } from "@/components/BotAvatar";
import { Icon } from "@/components/Icon";
import { Markdown } from "@/components/Markdown";
import { MenuItem } from "@/components/Popover";
import { toastError } from "@/components/Toast";
import { Badge, Empty, ErrorState, Loading, MoreMenu, PageHeader } from "@/components/ui";
import { useWindowFocused } from "@/features/agents/useSeen";
import type { Channel } from "@/gen/Channel";
import type { ChannelMessage } from "@/gen/ChannelMessage";
import type { ChannelPage as Page } from "@/gen/ChannelPage";
import { runAction } from "@/lib/actions";
import { useUserActive } from "@/lib/activity";
import { api } from "@/lib/api";
import { deliveryLines, loopFooter, mentionAt, type DeliveryLine } from "@/lib/channels";
import { ago, time, when } from "@/lib/format";
import { useKeptDraft } from "@/lib/keptDraft";
import { usePhone } from "@/lib/phone";
import { PhoneHeader } from "@/mobile/PhoneHeader";
import { phoneBack } from "@/mobile/back";

export function ChannelPage() {
  const { id } = useParams({ strict: false }) as { id: string };
  // One page per channel: nothing of the previous one (older pages, a draft) stays.
  return <ChannelPageOf key={id} id={id} />;
}

/** Older pages loaded by scrolling up, merged under the newest page (which the bus refreshes). */
type Older = { messages: ChannelMessage[]; deliveries: Page["deliveries"]; more: boolean | null };

function ChannelPageOf({ id }: { id: string }) {
  const phone = usePhone();
  const qc = useQueryClient();
  const navigate = useNavigate();
  const page = useQuery({ queryKey: ["channel", id], queryFn: () => api.get<Page>(`/api/channels/${id}`), retry: false });
  const [older, setOlder] = useState<Older>({ messages: [], deliveries: [], more: null });
  const [renaming, setRenaming] = useState(false);
  const channel = page.data?.channel;
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
  if (page.error || !channel) return <div className="flex h-full flex-col"><PageHeader title="Channel" /><ErrorState error={page.error ?? new Error("This channel no longer exists.")} onRetry={() => void page.refetch()} action={<Link to="/bots" className="btn btn-sm">Bots</Link>} /></div>;

  const memberNames = channel.members.map((m) => m.name);
  const state = channel.archived ? "Archived" : channel.blocked ? "Blocked: its bots cannot post" : channel.paused ? "Paused: messages post, nobody wakes up" : null;
  const meta = <span className="min-w-0 truncate" data-tour="channel.meta">{[state, channel.kind === "dm" ? null : channel.purpose || null, `${memberNames.join(", ") || "No bot"} · you`].filter(Boolean).join(" · ")}</span>;
  const refresh = () => Promise.all([qc.invalidateQueries({ queryKey: ["channel", id] }), qc.invalidateQueries({ queryKey: ["channels"] })]);
  const pause = <ActionButton name="channel.pause" params={{ id }} body={{ paused: !channel.paused }} icon={channel.paused ? "play" : "stop"} title={channel.paused ? "Deliveries start again" : "Stop every delivery in this channel: messages still post, nobody wakes up"} onDone={refresh}><span data-tour="channel.pause">{channel.paused ? "Resume" : "Pause"}</span></ActionButton>;
  const menu = (close: () => void) => <>
    {phone && <ActionMenuItem name="channel.pause" params={{ id }} body={{ paused: !channel.paused }} close={close} icon={channel.paused ? "play" : "stop"} label={channel.paused ? "Resume" : "Pause"} detail={channel.paused ? "Deliveries start again" : "Messages still post, nobody wakes up"} onDone={refresh} />}
    {channel.kind === "dm" && <ActionMenuItem name="channel.block" params={{ id }} body={{ blocked: !channel.blocked }} close={close} icon={channel.blocked ? "play" : "x"} label={channel.blocked ? "Unblock this DM" : "Block this DM"} detail={channel.blocked ? "Its bots can post again" : "Its bots can no longer post; you still can"} onDone={refresh} />}
    {channel.members.map((m) => <MenuItem key={m.bot_id} icon={<Icon name="comment" size={14} />} label={`Open ${m.name}'s chat`} detail="Its own thread, where it works on what it reads here" onClick={() => { close(); void navigate({ to: "/bots/$id", params: { id: m.bot_id } }); }} />)}
    {channel.kind === "channel" && <MenuItem icon={<Icon name="pencil" size={14} />} label="Rename" detail="Its name and purpose" onClick={() => { close(); setRenaming(true); }} />}
    {channel.kind === "channel" && <ActionMenuItem name="channel.archive" params={{ id }} body={{ archived: !channel.archived }} close={close} label={channel.archived ? "Bring back" : "Archive"} detail={channel.archived ? "Its bots can post again" : "Nobody posts; its history stays"} onDone={refresh} />}
    {channel.kind === "channel" && <ActionMenuItem name="channel.delete" params={{ id }} vars={{ name: channel.title }} close={close} onDone={async () => { await qc.invalidateQueries({ queryKey: ["channels"] }); void navigate({ to: "/bots" }); }} />}
  </>;
  const header = phone
    ? <PhoneHeader title={channel.title} onBack={() => phoneBack("/chats")} backLabel="All chats" subtitle={state ?? undefined} actions={menu} details={<p className="m-0 text-[12px] text-ink-3">{meta}</p>} />
    : <PageHeader title={channel.title} meta={meta} actions={<>{pause}<MoreMenu label={`More actions for ${channel.title}`} width={300}>{menu}</MoreMenu></>} />;
  return (
    <div className="flex h-full min-h-0 flex-col" data-tour="channel.page">
      {header}
      {renaming && <RenameForm channel={channel} onClose={() => setRenaming(false)} onDone={refresh} />}
      <MessageList channel={channel} messages={messages} lines={lines} byId={byId} more={more} loadingMore={loadingMore} onOlder={() => void loadOlder()} />
      {footer && <div className={`flex flex-wrap items-center gap-2 border-t border-line-2 px-5 py-1.5 text-[12px] ${footer.stopped ? "bg-attn-soft text-attn" : "text-ink-3"}`} data-tour="channel.guard" title="The loop guard: a bot's message counts one hop more than the message that woke it; your own messages reset the hop and the hourly count">
        <Icon name={footer.stopped ? "alert" : "refresh"} size={12} className="flex-none" />
        <span className="min-w-0 flex-1">{footer.stopped ? `${footer.stopped} · ` : ""}{footer.text}</span>
        {footer.stopped && <ActionButton name="channel.allow-more" params={{ id }} onDone={refresh} />}
      </div>}
      <ChannelComposer channel={channel} onSent={refresh} />
    </div>
  );
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

/** One message: its author's mark and name, the time, what it answers, its text and its deliveries. */
const MessageRow = memo(function MessageRow({ m, lines, reply, quoted }: { m: ChannelMessage; lines: DeliveryLine[] | undefined; reply: ChannelMessage | null; quoted: boolean }) {
  const owner = m.author.kind === "owner";
  const mentionsYou = !owner && m.mentions.includes("owner");
  const today = new Date(m.created_at).toDateString() === new Date().toDateString();
  return <div className="flex items-start gap-2.5 py-2" data-tour="channel.message">
    {owner ? <span className="grid size-7 flex-none place-items-center rounded-card bg-sunk text-[13px] font-semibold text-ink-2" aria-hidden>Y</span> : <BotAvatar name={m.author.name} size={28} />}
    <div className="grid min-w-0 flex-1 gap-0.5">
      <div className="flex min-w-0 flex-wrap items-center gap-x-1.5 text-[12px] text-ink-3">
        {m.author.kind === "bot" ? <Link to="/bots/$id" params={{ id: m.author.id }} className={`font-semibold text-ink ${NAME_LINK}`}>{m.author.name}</Link> : <b className="font-semibold text-ink">{owner ? "You" : m.author.name}</b>}
        <span title={`${when(m.created_at)} · message ${m.id} · hop ${m.hops}`}>{today ? time(m.created_at) : ago(m.created_at)}</span>
        {m.ask_owner ? <Badge tone="attn">Asks you</Badge> : mentionsYou ? <Badge tone="attn">Mentions you</Badge> : null}
      </div>
      {quoted && <div className="truncate text-[12px] text-ink-3" title={reply ? reply.text : undefined}>↳ Reply to {reply ? `${reply.author.kind === "owner" ? "you" : reply.author.name}: ${firstLine(reply.text)}` : `message ${m.reply_to}`}</div>}
      <Markdown text={m.text} className="text-[13px]" />
      {lines?.map((l, i) => <Delivery key={i} l={l} />)}
    </div>
  </div>;
}, (a, b) => a.m === b.m && a.reply === b.reply && a.quoted === b.quoted && JSON.stringify(a.lines) === JSON.stringify(b.lines));

/** A delivery as one muted line: "Maintainer woke up (mention) · open its chat", "Docs is busy · message queued (1)". */
function Delivery({ l }: { l: DeliveryLine }) {
  const chat = <Link to="/bots/$id" params={{ id: l.botId }} className="text-teal-ink no-underline hover:underline" title="Its own chat, where it answers and works">open its chat</Link>;
  const row = "flex min-w-0 items-start gap-1.5 text-[12px]";
  switch (l.kind) {
    case "woke":
      return <div className={`${row} text-ink-3`} data-tour="channel.delivery"><Icon name="play" size={12} className="mt-0.5 flex-none" /><span className="min-w-0 break-words">{l.bot} woke up ({l.why}) · {chat}</span></div>;
    case "queued":
      return <div className={`${row} text-ink-3`} title={l.reason ?? undefined}><Icon name="clock" size={12} className="mt-0.5 flex-none" /><span className="min-w-0 break-words">{l.bot} is busy · message queued ({l.waiting})</span></div>;
    case "combined":
      return <div className={`${row} text-ink-3`}><Icon name="collapse" size={12} className="mt-0.5 flex-none" /><span className="min-w-0 break-words">{l.bot}: combined with {l.with != null ? `message ${l.with}` : "an earlier message"}{l.session ? <> · {chat}</> : null}</span></div>;
    case "skipped":
      return <div className={`${row} ${l.loop ? "text-attn" : "text-ink-3"}`} title={l.reason}><Icon name={l.loop ? "alert" : "x"} size={12} className="mt-0.5 flex-none" /><span className="min-w-0 break-words">{l.bot} skipped: {l.reason}</span></div>;
  }
}

// ---------------------------------------------------------------- rename

function RenameForm({ channel, onClose, onDone }: { channel: Channel; onClose: () => void; onDone: () => unknown }) {
  const [name, setName] = useState(channel.name);
  const [purpose, setPurpose] = useState(channel.purpose);
  const [busy, setBusy] = useState(false);
  const save = async () => {
    setBusy(true);
    try { await runAction("channel.rename", { id: channel.id }, { name: name.trim(), purpose: purpose.trim() }); await onDone(); onClose(); } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  return <div className="flex flex-wrap items-end gap-2 border-b border-line-2 px-5 py-2">
    <label className="grid gap-1 text-[12px] font-semibold text-ink-2">Name<input className="field-input font-normal" value={name} maxLength={40} onChange={(e) => setName(e.target.value)} autoFocus /></label>
    <label className="grid min-w-0 flex-1 gap-1 text-[12px] font-semibold text-ink-2">Purpose<input className="field-input font-normal" value={purpose} maxLength={200} onChange={(e) => setPurpose(e.target.value)} /></label>
    <button className="btn btn-sm btn-ghost" onClick={onClose}>Cancel</button>
    <button className="btn btn-sm btn-primary" disabled={busy || !name.trim()} onClick={() => void save()}>Save</button>
  </div>;
}

// ---------------------------------------------------------------- the message box

/** Your message: Enter sends, Shift+Enter adds a line; `@` suggests the member bots (Enter or Tab picks the first). */
function ChannelComposer({ channel, onSent }: { channel: Channel; onSent: () => unknown }) {
  const phone = usePhone();
  const { text, setText, clear } = useKeptDraft(`channel.${channel.id}`, "");
  const [busy, setBusy] = useState(false);
  const [caret, setCaret] = useState(0);
  const [dismissed, setDismissed] = useState<number | null>(null);
  const ref = useRef<HTMLTextAreaElement>(null);
  useLayoutEffect(() => { const el = ref.current; if (!el) return; el.style.height = "auto"; el.style.height = `${Math.min(el.scrollHeight, 220)}px`; }, [text]);
  const at = mentionAt(text, caret);
  const suggestions = at && at.start !== dismissed ? channel.members.filter((m) => m.slug && (m.slug.startsWith(at.query) || m.name.toLowerCase().startsWith(at.query))).slice(0, 6) : [];
  const pick = (slug: string) => {
    if (!at) return;
    const next = `${text.slice(0, at.start)}@${slug} ${text.slice(caret)}`;
    const pos = at.start + slug.length + 2;
    setText(next);
    setCaret(pos);
    requestAnimationFrame(() => { const el = ref.current; if (el) { el.focus(); el.setSelectionRange(pos, pos); } });
  };
  const send = async () => {
    const t = text.trim();
    if (!t || busy) return;
    setBusy(true);
    try {
      await runAction("channel.post", { id: channel.id }, { text: t });
      setText("");
      clear();
      await onSent();
    } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  const dm = channel.kind === "dm";
  const closed = channel.archived;
  const name = dm ? "both bots" : channel.title;
  return (
    <div className="border-t border-line-2 px-5 pt-2 pb-2.5" data-tour="channel.reply">
      <div className="relative w-full max-w-text">
        {dm && !closed && <div className="mb-1.5 text-[12px] text-ink-3">Both bots will see your message</div>}
        {closed && <div className="mb-1.5 text-[12px] text-ink-3">Archived: nobody posts here. ⋯ → Bring back to post again.</div>}
        {suggestions.length > 0 && <div role="listbox" aria-label="Mention a bot" className="absolute bottom-full left-0 z-10 mb-1 grid min-w-[220px] gap-px rounded-card border border-line bg-paper p-1 shadow-[0_8px_24px_-12px_rgba(23,32,43,.5)]">
          {suggestions.map((m, i) => <button key={m.bot_id} role="option" aria-selected={i === 0} className={`flex ${phone ? "min-h-11" : "min-h-8"} items-center gap-2 rounded-control px-2 text-left text-[13px] hover:bg-sunk ${i === 0 ? "bg-sunk" : ""}`} onMouseDown={(e) => e.preventDefault()} onClick={() => pick(m.slug)}>
            <BotAvatar name={m.name} size={18} /><span className="min-w-0 flex-1 truncate">{m.name}</span><span className="text-[12px] text-ink-3">@{m.slug}</span>
          </button>)}
        </div>}
        <div className="session-message-input flex items-end gap-2 rounded-control border border-line px-2.5 py-[7px] focus-within:border-teal focus-within:shadow-[0_0_0_3px_var(--color-teal-soft)]">
          <textarea ref={ref} rows={1} value={text} disabled={busy || closed} aria-label={`Message ${name}`}
            placeholder={dm ? "Message both bots…" : `Message ${channel.title}… (@ to wake a bot)`}
            onChange={(e) => { setText(e.target.value); setCaret(e.target.selectionStart); setDismissed(null); }}
            onSelect={(e) => setCaret(e.currentTarget.selectionStart)}
            onKeyDown={(e) => {
              if (suggestions.length && !e.nativeEvent.isComposing && (e.key === "Tab" || (e.key === "Enter" && !e.shiftKey))) { e.preventDefault(); pick(suggestions[0].slug); return; }
              if (suggestions.length && e.key === "Escape") { e.preventDefault(); setDismissed(at?.start ?? null); return; }
              if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) { e.preventDefault(); void send(); }
            }}
            className="min-h-6 min-w-0 flex-1 resize-none bg-transparent text-[13px] leading-5 outline-none" />
          <button className="iconbtn border-line! bg-paper" title="Send" aria-label="Send" disabled={busy || closed || !text.trim()} onClick={() => void send()}><Icon name="send" size={14} /></button>
        </div>
      </div>
    </div>
  );
}
