// The phone's Chats tab: every bot of the workspace as a conversation, the Ask bot pinned first, then the others by
// their last activity; then the workspace's channels and bot DMs, each with its last message. A tap opens the chat or
// the channel; ＋ New creates a bot or a channel; Archived channels lists the archived ones. Tapping the tab again opens
// the last chat.

import { Link, useNavigate } from "@tanstack/react-router";
import { Icon } from "@/components/Icon";
import { MenuItem, Popover } from "@/components/Popover";
import { Empty, ErrorState, Loading } from "@/components/ui";
import { BotConversationRow } from "@/features/bots/BotConversationRow";
import { ChannelConversationRow } from "@/features/channels/ChannelRows";
import { conversationOrder, useBots } from "@/lib/bots";
import { archivedOf, channelsOf, useChannels } from "@/lib/channels";
import { useSessions } from "@/lib/queries";
import { useScope } from "@/lib/workspace";
import { PhoneBarEnd } from "./PhoneHeader";

export function ChatsView() {
  const scope = useScope();
  const bots = useBots();
  const sessions = useSessions().data ?? [];
  const rows = conversationOrder(bots.data ?? [], sessions, scope.workspace.id);
  const all = useChannels().data;
  const { channels, dms } = channelsOf(all, scope.workspace.id);
  const archived = archivedOf(all, scope.workspace.id).length;
  const navigate = useNavigate();
  const label = "seclabel m-0 border-b border-line-2 px-3 pt-3 pb-1";
  return <div className="h-full overflow-y-auto" data-tour="phone.chats">
    <PhoneBarEnd>{rows.length === 0 ? <Link to="/bots" search={{ new: true } as never} className="btn btn-sm"><Icon name="plus" size={12} />New bot</Link>
      : <Popover viewport align="right" width={260} label="New" trigger={({ open, toggle }) => <button className="btn btn-sm" aria-expanded={open} aria-haspopup="dialog" data-tour="phone.chats-new" onClick={toggle}><Icon name="plus" size={12} />New</button>}>
        {(close) => <div className="grid gap-px">
          <MenuItem icon={<Icon name="spark" size={14} />} label="New bot" detail="An agent with its own chat" onClick={() => { close(); void navigate({ to: "/bots", search: { new: true } as never }); }} />
          <MenuItem icon={<Icon name="comment" size={14} />} label="New channel" detail="A conversation of some bots, and you" onClick={() => { close(); void navigate({ to: "/bots", search: { new: "channel" } as never }); }} />
        </div>}
      </Popover>}</PhoneBarEnd>
    {bots.isLoading ? <Loading what="bots" /> : bots.error ? <ErrorState error={bots.error} onRetry={() => void bots.refetch()} />
      : rows.length === 0 ? <Empty icon="spark" action={<Link to="/bots" search={{ new: true } as never} className="btn btn-sm btn-primary">New bot</Link>}>{`No bot in ${scope.workspace.name} yet.`}</Empty>
      : <nav aria-label="Chats">
        {(channels.length > 0 || dms.length > 0) && <h2 className={label}>Bots</h2>}
        {rows.map(({ bot, thread, pinned }) => <BotConversationRow key={bot.id} bot={bot} thread={thread} pinned={pinned} className="border-b border-line-2" />)}
        {channels.length > 0 && <><h2 className={label} data-tour="phone.channels">Channels</h2>{channels.map((c) => <ChannelConversationRow key={c.id} c={c} className="border-b border-line-2" />)}</>}
        {dms.length > 0 && <><h2 className={label}>Bot DMs</h2>{dms.map((c) => <ChannelConversationRow key={c.id} c={c} className="border-b border-line-2" />)}</>}
        {archived > 0 && <Link to="/bots" search={{ archived: true } as never} className="flex min-h-11 items-center gap-2 px-3 text-[13px] text-ink-3 no-underline" data-tour="phone.archived"><Icon name="archive" size={14} /><span className="flex-1">Archived channels</span><span>{archived}</span></Link>}
      </nav>}
  </div>;
}
