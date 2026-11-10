// The phone's Chats tab: every bot of the workspace as a conversation, the Ask bot pinned first, then the others by
// their last activity. A tap opens the bot's chat; ＋ creates a bot. Tapping the tab again opens the last chat.

import { Link } from "@tanstack/react-router";
import { Icon } from "@/components/Icon";
import { Empty, ErrorState, Loading } from "@/components/ui";
import { BotConversationRow } from "@/features/bots/BotConversationRow";
import { conversationOrder, useBots } from "@/lib/bots";
import { useSessions } from "@/lib/queries";
import { useScope } from "@/lib/workspace";
import { PhoneBarEnd } from "./PhoneHeader";

export function ChatsView() {
  const scope = useScope();
  const bots = useBots();
  const sessions = useSessions().data ?? [];
  const rows = conversationOrder(bots.data ?? [], sessions, scope.workspace.id);
  return <div className="h-full overflow-y-auto" data-tour="phone.chats">
    <PhoneBarEnd><Link to="/bots" search={{ new: true } as never} className="btn btn-sm"><Icon name="plus" size={12} />New bot</Link></PhoneBarEnd>
    {bots.isLoading ? <Loading what="bots" /> : bots.error ? <ErrorState error={bots.error} onRetry={() => void bots.refetch()} />
      : rows.length === 0 ? <Empty icon="spark" action={<Link to="/bots" search={{ new: true } as never} className="btn btn-sm btn-primary">New bot</Link>}>{`No bot in ${scope.workspace.name} yet.`}</Empty>
      : <nav aria-label="Chats">{rows.map(({ bot, thread, pinned }) => <BotConversationRow key={bot.id} bot={bot} thread={thread} pinned={pinned} className="border-b border-line-2" />)}</nav>}
  </div>;
}
