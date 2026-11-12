// A bot channel or DM in a list: the sidebar's Bots group (ChannelLink), and the Bots page and the phone's Chats tab
// (ChannelConversationRow, with its last message and time). A plain dot shows unread messages; an amber count only
// the unread ones that mention you or ask you (`--ask-owner`), read from the channel's Today row.

import { useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { memo } from "react";
import { ActionButton, ActionMenuItem } from "@/components/ActionButton";
import { Count, RowActions } from "@/components/ui";
import type { Channel } from "@/gen/Channel";
import { useChannelAsks } from "@/features/today/data";
import { channelLastLine, channelMark } from "@/lib/channels";
import { age } from "@/lib/format";

/** The unread mark of a row: an amber count when a bot asks you there (its Today row), else a plain dot. */
function Unread({ c }: { c: Channel }) {
  const asks = useChannelAsks().get(c.id) ?? 0;
  if (asks > 0) return <Count n={asks} needs title={`${asks} message${asks === 1 ? "" : "s"} for you: answer from Today or here`} />;
  if (c.unread > 0) return <span className="size-[7px] flex-none rounded-full bg-ink-3" title={`${c.unread} unread`} />;
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
  return <Link to="/channels/$id" params={{ id: c.id }} className={`grid min-h-[60px] grid-cols-[36px_minmax(0,1fr)_auto] items-center gap-x-3 px-3 py-2 no-underline hover:bg-sunk ${className}`} aria-label={`${c.title}${c.unread ? `, ${c.unread} unread` : ""}`}>
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

/** An archived channel or DM (Bots → Archived channels): its last message, Bring back, and Delete in ⋯. */
export const ArchivedChannelRow = memo(function ArchivedChannelRow({ c, className = "" }: { c: Channel; className?: string }) {
  const qc = useQueryClient();
  const refresh = () => qc.invalidateQueries({ queryKey: ["channels"] });
  return <div className={`grid min-h-[60px] grid-cols-[36px_minmax(0,1fr)_auto] items-center gap-x-3 px-3 py-2 ${className}`}>
    <span className="grid size-9 place-items-center rounded-card bg-sunk text-[15px] font-semibold text-ink-3" aria-hidden>{channelMark(c)}</span>
    <Link to="/channels/$id" params={{ id: c.id }} className="grid min-w-0 gap-0.5 no-underline">
      <span className="truncate text-[13px] font-medium text-ink">{c.kind === "dm" ? c.title : c.name}</span>
      <span className="truncate text-[12px] text-ink-3">{c.last_at ? `Last message ${age(c.last_at)} · ` : ""}{channelLastLine(c)}</span>
    </Link>
    <RowActions label={`More actions for ${c.title}`}
      main={<ActionButton name="channel.archive" params={{ id: c.id }} body={{ archived: false }} icon="refresh" title="Its bots can read and post again" onDone={refresh}>Bring back</ActionButton>}
      more={(close) => <ActionMenuItem name="channel.delete" params={{ id: c.id }} vars={{ name: c.title }} close={close} onDone={refresh} />} />
  </div>;
});
