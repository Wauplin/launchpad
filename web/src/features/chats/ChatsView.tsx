// Chats: the Agents layout for bots. On the laptop a resizable list of every chat (`ChatList`) on the left and the
// main frame on the right, which shows the open chat, a bot's or channel's settings, or, with nothing open, the
// landing grid of every bot and channel (alphabetical; a bot's tile starts a new chat, a channel's opens it, the gear
// of each tile opens its settings). `ChatsFrame` is a layout route, so the list stays mounted while chats open and
// close. On the phone, the Chats tab is the list; New chat opens the grid (`?view=start`), and a chat or settings page
// replaces the list, with ‹ back to it.

import { Link, Outlet, useNavigate, useSearch } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { BotAvatar } from "@/components/BotAvatar";
import { Icon } from "@/components/Icon";
import { MenuItem } from "@/components/Popover";
import { Empty, ErrorState, Loading, PageHeader } from "@/components/ui";
import { useColumn } from "@/components/ResizeHandle";
import { NewBot } from "@/features/bots/BotsView";
import { NewChannel } from "@/features/channels/NewChannel";
import { useBots } from "@/lib/bots";
import { byName, channelMark, channelName, channelsOf, useChannels } from "@/lib/channels";
import { usePhone } from "@/lib/phone";
import { useScope } from "@/lib/workspace";
import { PhoneBarEnd, PhoneHeader } from "@/mobile/PhoneHeader";
import { phoneBack } from "@/mobile/back";
import { ChatList } from "./ChatList";

export type ChatsSearch = { new?: true | "channel"; view?: "start" };

/** The layout of every Chats page: the list and the main frame on the laptop, the page alone on the phone. */
export function ChatsFrame() {
  const phone = usePhone();
  const column = useColumn("chats-list", "chat list");
  if (phone) return <Outlet />;
  return <div className="flex h-full min-h-0" data-tour="chats.page">
    <div className="flex flex-none flex-col" style={{ width: column.width }}><ChatList /></div>
    {column.handle}
    <div className="flex min-h-0 min-w-0 flex-1 flex-col bg-paper"><Outlet /></div>
  </div>;
}

/** `/chats`: the landing grid (laptop) or the list (phone); `?new=` holds the New bot and New channel forms, and
 * `?view=start` the phone's grid. */
export function ChatsView() {
  const phone = usePhone();
  const search = useSearch({ strict: false }) as ChatsSearch;
  const bots = useBots();
  const scope = useScope();
  const mine = (bots.data ?? []).filter((b) => b.workspace_id === scope.workspace.id);
  const newChannel = search.new === "channel" && mine.length > 0;
  const creating = !newChannel && (search.new === true || (bots.isSuccess && mine.length === 0));
  if (phone && !creating && !newChannel && search.view !== "start") {
    return <div className="h-full min-h-0" data-tour="phone.chats">
      <PhoneBarEnd><Link to="/chats" search={{ view: "start" } as never} className="btn btn-sm" data-tour="phone.chats-new"><Icon name="plus" size={12} />New chat</Link></PhoneBarEnd>
      <ChatList />
    </div>;
  }
  return <Landing />;
}

/** A tile's link covers the whole tile (keyboard focus included); the gear sits over it. */
const TILE = "absolute inset-0 rounded-card focus-visible:outline-2 focus-visible:outline-teal";

type Tile = { id: string; name: string; mark: ReactNode; open: ReactNode; settings: ReactNode; bold?: boolean; detail?: string };

/** One section of the landing grid: a tile per bot or channel, its mark and name, the gear in its top-right corner. */
function LandingGrid({ label, tour, items }: { label: string; tour: string; items: Tile[] }) {
  return <section className="grid gap-2" aria-label={label} data-tour={tour}>
    <h2 className="seclabel m-0">{label}</h2>
    <ul className="m-0 grid list-none grid-cols-[repeat(auto-fill,minmax(150px,1fr))] gap-2 p-0">
      {items.map((it) => <li key={it.id} className={`relative flex min-h-[96px] flex-col items-center justify-center gap-2 rounded-card bg-paper p-3 text-center hover:bg-sunk ${it.bold ? "border-2 border-ink" : "border border-line"}`} data-tour="chats.tile">
        <span aria-hidden>{it.mark}</span>
        <span className={`w-full truncate px-4 text-[13px] text-ink ${it.bold ? "font-bold" : "font-medium"}`} title={it.name}>{it.name}</span>
        {it.detail && <span className={`-mt-1.5 text-[11px] ${it.bold ? "font-semibold text-ink-2" : "text-ink-3"}`}>{it.detail}</span>}
        {it.open}
        <span className="absolute top-1 right-1 z-10">{it.settings}</span>
      </li>)}
    </ul>
  </section>;
}

const gear = "iconbtn text-ink-3 no-underline hover:text-ink";

/** Every bot and channel of the workspace, alphabetical, to start or open a chat; New bot and New channel. */
function Landing() {
  const phone = usePhone();
  const navigate = useNavigate();
  const search = useSearch({ strict: false }) as ChatsSearch;
  const bots = useBots();
  const scope = useScope();
  const mine = (bots.data ?? []).filter((b) => b.workspace_id === scope.workspace.id);
  const all = useChannels().data;
  const { channels, dms } = channelsOf(all, scope.workspace.id);
  const newChannel = search.new === "channel" && mine.length > 0;
  const creating = !newChannel && (search.new === true || (bots.isSuccess && mine.length === 0));
  // The forms and the phone's grid are pages over the list: ‹ and Cancel go back.
  const close = () => { if (phone) phoneBack(creating || newChannel ? "/chats?view=start" : "/chats"); else void navigate({ to: "/chats", search: {} as never }); };
  const newBotLink = <Link to="/chats" search={{ new: true } as never} className={`btn btn-sm ${phone ? "" : "btn-primary"}`} data-tour="bots.new"><Icon name="plus" size={12} />New bot</Link>;
  const newChannelLink = mine.length > 0 && <Link to="/chats" search={{ new: "channel" } as never} className="btn btn-sm" data-tour="channels.new"><Icon name="plus" size={12} />New channel</Link>;
  const title = newChannel ? "New channel" : creating ? (mine.length ? "New bot" : "Chats") : phone ? "New chat" : "Chats";
  const header = phone
    ? <PhoneHeader title={title} onBack={() => phoneBack("/chats")} backLabel="All chats" primary={!creating && !newChannel ? newBotLink : undefined}
      actions={!creating && !newChannel && mine.length > 0 ? (closeMenu: () => void) => <MenuItem icon={<Icon name="comment" size={14} />} label="New channel" detail="A conversation of some bots, and you" onClick={() => { closeMenu(); void navigate({ to: "/chats", search: { new: "channel" } as never }); }} /> : undefined} />
    : <PageHeader title={title} meta={mine.length ? [`${mine.length} bot${mine.length === 1 ? "" : "s"}`, channels.length && `${channels.length} channel${channels.length === 1 ? "" : "s"}`, dms.length && `${dms.length} DM${dms.length === 1 ? "" : "s"}`].filter(Boolean).join(" · ") : undefined}
      intro="Start a chat with a bot, or open a channel. Your chats are on the left, by state; the gear of a tile opens its settings." actions={!creating && !newChannel && <>{newChannelLink}{newBotLink}</>} />;
  const tiles: Tile[] = byName(mine, (b) => b.settings.name).map((b) => ({
    id: b.id, name: b.settings.name, detail: b.paused ? "Triggers paused" : undefined,
    mark: <BotAvatar name={b.settings.name} id={b.id} size={36} />,
    open: <Link to="/bots/$id" params={{ id: b.id }} search={{ thread: "new" } as never} className={TILE} aria-label={`New chat with ${b.settings.name}`} title={`Start a new chat with ${b.settings.name}`} />,
    settings: <Link to="/bots/$id/settings" params={{ id: b.id }} className={gear} aria-label={`Settings of ${b.settings.name}`} title={`Settings of ${b.settings.name}: BOT.md, triggers, memory, access, versions`}><Icon name="gear" size={14} /></Link>,
  }));
  const channelTiles: Tile[] = [...byName(channels, channelName), ...byName(dms, channelName)].map((c) => ({
    id: c.id, name: channelName(c), bold: c.unread > 0, detail: c.unread > 0 ? `${c.unread} unread` : c.paused ? "Paused" : c.owner_muted ? "Muted" : c.kind === "dm" ? "DM" : undefined,
    mark: <span className="grid size-9 place-items-center rounded-card border border-line bg-sunk text-[18px] font-semibold text-ink-2">{channelMark(c)}</span>,
    open: <Link to="/channels/$id" params={{ id: c.id }} className={TILE} aria-label={`Open ${c.kind === "dm" ? "DM" : "channel"} ${channelName(c)}${c.unread ? `, ${c.unread} unread` : ""}`} />,
    settings: <Link to="/channels/$id/settings" params={{ id: c.id }} className={gear} aria-label={`Settings of ${channelName(c)}`} title={`Settings of ${channelName(c)}: ${c.kind === "dm" ? "pause or block" : "members, name, pause, archive"}`}><Icon name="gear" size={14} /></Link>,
  }));
  return <div className="flex h-full min-h-0 flex-col" data-tour="chats.landing">
    {header}
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="grid w-full max-w-page grid-cols-[minmax(0,1fr)] gap-4 px-5 py-4">
        {bots.isLoading ? <Loading what="bots" /> : bots.error ? <ErrorState error={bots.error} onRetry={() => void bots.refetch()} /> : <>
          {creating && <NewBot first={mine.length === 0} onCancel={mine.length ? close : undefined} />}
          {newChannel && <NewChannel onCancel={close} />}
          {!creating && !newChannel && <>
            {tiles.length ? <LandingGrid label="Bots" tour="bots.grid" items={tiles} /> : <Empty icon="spark">No bot yet.</Empty>}
            {channelTiles.length > 0 && <LandingGrid label="Channels" tour="bots.channels" items={channelTiles} />}
          </>}
        </>}
      </div>
    </div>
  </div>;
}
