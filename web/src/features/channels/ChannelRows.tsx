// A bot channel or DM in a list: the sidebar's Bots group (ChannelLink), and the Bots page and the phone's Chats tab
// (ChannelConversationRow, with its last message and time). A plain dot shows unread messages; an amber count only
// the unread ones that mention you or ask you (`--ask-owner`).

import { Link } from "@tanstack/react-router";
import { memo } from "react";
import { Count } from "@/components/ui";
import type { Channel } from "@/gen/Channel";
import { channelLastLine, channelMark } from "@/lib/channels";
import { age } from "@/lib/format";

const unreadTitle = (c: Channel) => c.attention ? `${c.attention} message${c.attention === 1 ? "" : "s"} for you` : `${c.unread} unread`;

/** The unread mark of a row: an amber count for messages that need you, else a plain dot. */
function Unread({ c }: { c: Channel }) {
  if (c.attention > 0) return <Count n={c.attention} needs title={unreadTitle(c)} />;
  if (c.unread > 0) return <span className="size-[7px] flex-none rounded-full bg-ink-3" title={unreadTitle(c)} />;
  return null;
}

/** A sidebar row, under the bots. */
export const ChannelLink = memo(function ChannelLink({ c }: { c: Channel }) {
  return (
    <Link to="/channels/$id" params={{ id: c.id }} className="flex h-[28px] items-center gap-2 rounded-control px-[9px] text-[13px] text-ink no-underline hover:bg-sunk" activeProps={{ className: "!bg-paper shadow-[inset_0_0_0_1px_var(--color-line)] font-semibold" }}>
      <span className="grid size-[16px] flex-none place-items-center text-[12px] text-ink-3" aria-hidden>{channelMark(c)}</span>
      <span className={`min-w-0 flex-1 truncate ${c.unread ? "font-semibold" : ""}`}>{c.kind === "dm" ? c.title : c.name}</span>
      <Unread c={c} />
    </Link>
  );
});

/** A conversation row (Bots page, phone Chats): its mark, title, last message and time. */
export const ChannelConversationRow = memo(function ChannelConversationRow({ c, className = "" }: { c: Channel; className?: string }) {
  const line = channelLastLine(c);
  return <Link to="/channels/$id" params={{ id: c.id }} className={`grid min-h-[60px] grid-cols-[36px_minmax(0,1fr)_auto] items-center gap-x-3 px-3 py-2 no-underline hover:bg-sunk ${className}`} aria-label={`${c.title}${c.unread ? `, ${unreadTitle(c)}` : ""}`}>
    <span className="grid size-9 place-items-center rounded-card bg-sunk text-[15px] font-semibold text-ink-2" aria-hidden>{channelMark(c)}</span>
    <span className="grid min-w-0 gap-0.5">
      <span className={`flex min-w-0 items-center gap-1.5 text-[13px] text-ink ${c.unread ? "font-semibold" : "font-medium"}`}>
        <span className="truncate">{c.kind === "dm" ? c.title : c.name}</span>{c.paused && <span className="shrink-0 text-[11px] font-normal text-ink-3">Paused</span>}{c.blocked && <span className="shrink-0 text-[11px] font-normal text-ink-3">Blocked</span>}
      </span>
      <span className={`truncate text-[12px] ${c.unread ? "font-semibold text-ink-2" : "text-ink-3"}`} title={line}>{line}</span>
    </span>
    <span className="grid justify-items-end gap-1 text-[12px] text-ink-3">
      <span>{c.last_at ? age(c.last_at) : ""}</span>
      <Unread c={c} />
    </span>
  </Link>;
});
