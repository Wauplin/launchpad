// A bot's threads: its parallel conversations (the owner's main one, one per GitHub item, channel, hook or agent,
// one per routine run, and the owner's new threads). The bot page shows one thread at a time (`?thread=`, the only
// place that state lives); switching replaces the history entry, like tabs. On the laptop a bar of chips under the
// header (main and the most recent threads, each with its state dot, then All threads with a search); on the
// phone a sheet from the chat's bar. Rename and Archive are in the page's ⋯.

import { useQueryClient } from "@tanstack/react-query";
import { memo, useMemo, useState } from "react";
import type { Bot } from "@/gen/Bot";
import type { BotThread } from "@/gen/BotThread";
import type { Session } from "@/gen/Session";
import { ActionButton } from "@/components/ActionButton";
import { Icon } from "@/components/Icon";
import { Popover } from "@/components/Popover";
import { FilterChip, FilterChips } from "@/components/ui";
import { botThread, MAIN_THREAD, threadState, useBotThreads, type BotState } from "@/lib/bots";
import { age } from "@/lib/format";

/** The `?thread=` value of a thread the owner starts with their next message. */
export const NEW_THREAD = "new";

const DOT: Record<BotState, string> = { needs: "bg-attn", running: "bg-work", paused: "bg-attn", idle: "bg-line", new: "bg-line" };
const STATE_LABEL: Record<BotState, string> = { needs: "Needs you", running: "Working", paused: "Quota paused", idle: "Idle", new: "No message yet" };

/** A thread with its state and last activity, from the bot's sessions. */
export type ThreadRow = { t: BotThread; state: BotState; last: number };

/** The threads of a bot with their states: main first, then by last activity, newest first. */
export function useThreadRows(bot: Bot, sessions: Session[]): ThreadRow[] {
  const threads = useBotThreads(bot.id).data;
  return useMemo(() => {
    const rows = (threads ?? []).map((t): ThreadRow => {
      const mine = botThread(sessions, bot.id, t.id);
      return { t, state: threadState(mine), last: mine.at(-1)?.status_since ?? t.created_at };
    });
    return rows.sort((a, b) => Number(b.t.id === MAIN_THREAD) - Number(a.t.id === MAIN_THREAD) || b.last - a.last);
  }, [threads, sessions, bot.id]);
}

export const ThreadDot = ({ state }: { state: BotState }) => <span className={`size-2 flex-none rounded-full ${DOT[state]}`} title={STATE_LABEL[state]} aria-hidden />;

/** Chips shown in the laptop bar besides main and the selected thread. */
const RECENT = 4;

/** The laptop's thread bar under the header: main and the most recent threads, then All threads. */
export function ThreadBar({ bot, rows, selected, onSelect }: { bot: Bot; rows: ThreadRow[]; selected: string; onSelect: (thread: string) => void }) {
  const shown = useMemo(() => {
    const [main, ...rest] = rows;
    const recent = rest.slice(0, RECENT);
    const sel = rest.find((r) => r.t.id === selected);
    return [main, ...recent, ...(sel && !recent.includes(sel) ? [sel] : [])].filter(Boolean) as ThreadRow[];
  }, [rows, selected]);
  return <div className="flex min-h-10 flex-none items-center gap-1.5 border-b border-line-2 px-5 py-1.5" data-tour="bots.threads">
    <FilterChips label={`Threads of ${bot.settings.name}`} className="min-w-0 flex-1 flex-nowrap overflow-hidden">
      {shown.map((r) => <FilterChip key={r.t.id} active={r.t.id === selected} title={`${r.t.title} · ${STATE_LABEL[r.state]}`} onClick={() => onSelect(r.t.id)}>
        <ThreadDot state={r.state} /><span className="max-w-[180px] truncate">{r.t.title}</span>
      </FilterChip>)}
      {selected === NEW_THREAD && <FilterChip active onClick={() => {}}><Icon name="plus" size={12} />New thread</FilterChip>}
    </FilterChips>
    <Popover align="right" width={340} viewport label="All threads" trigger={({ open, toggle }) => <button className="btn btn-sm flex-none" aria-expanded={open} aria-haspopup="dialog" onClick={toggle}>All threads ({rows.length})</button>}>
      {(close) => <ThreadList bot={bot} rows={rows} selected={selected} onSelect={(t) => { close(); onSelect(t); }} />}
    </Popover>
  </div>;
}

/** Every thread with a search (the laptop's All threads, the phone's Threads sheet), New thread first, then the
 * archived threads behind a toggle, each with Bring back. */
export function ThreadList({ bot, rows, selected, onSelect }: { bot: Bot; rows: ThreadRow[]; selected: string; onSelect: (thread: string) => void }) {
  const [query, setQuery] = useState("");
  const [archived, setArchived] = useState(false);
  const old = useBotThreads(archived ? bot.id : null, true).data?.filter((t) => t.archived_at != null) ?? [];
  const q = query.trim().toLowerCase();
  const list = q ? rows.filter((r) => r.t.title.toLowerCase().includes(q)) : rows;
  const qc = useQueryClient();
  return <div className="grid gap-1 p-1.5">
    {rows.length > 6 && <input className="field-input" placeholder="Search threads" aria-label="Search threads" value={query} onChange={(e) => setQuery(e.target.value)} autoFocus />}
    <button className="flex min-h-9 items-center gap-2 rounded-control px-2 text-left text-[13px] hover:bg-sunk" aria-pressed={selected === NEW_THREAD} onClick={() => onSelect(NEW_THREAD)} data-tour="bots.new-thread">
      <Icon name="plus" size={12} className="flex-none text-ink-3" /><span className="min-w-0 flex-1 truncate">New thread</span><span className="text-[12px] text-ink-3">your next message starts it</span>
    </button>
    <div className="grid max-h-[50vh] gap-px overflow-y-auto">
      {list.map((r) => <ThreadItem key={r.t.id} r={r} active={r.t.id === selected} onSelect={onSelect} />)}
      {list.length === 0 && <p className="m-0 px-2 py-1.5 text-[12px] text-ink-3">No thread matches.</p>}
    </div>
    <button className="chat-tap justify-self-start px-2 text-[12px] text-teal-ink hover:underline" aria-expanded={archived} onClick={() => setArchived(!archived)}>{archived ? "Hide archived threads" : "Show archived threads"}</button>
    {archived && <div className="grid gap-px">
      {old.length === 0 ? <p className="m-0 px-2 py-1 text-[12px] text-ink-3">No archived thread.</p>
        : old.map((t) => <div key={t.id} className="flex min-h-9 items-center gap-2 px-2 text-[13px] text-ink-3">
          <Icon name="archive" size={12} className="flex-none" /><span className="min-w-0 flex-1 truncate" title={t.title}>{t.title}</span>
          <ActionButton name="bot.thread-archive" params={{ id: bot.id, thread: t.id }} body={{ archived: false }} consequence="Brings the thread back to the bar; its archived sessions stay archived." confirmLabel="Bring back" icon={null} onDone={() => qc.invalidateQueries({ queryKey: ["bot-threads"] })}>Bring back</ActionButton>
        </div>)}
    </div>}
  </div>;
}

const ThreadItem = memo(function ThreadItem({ r, active, onSelect }: { r: ThreadRow; active: boolean; onSelect: (thread: string) => void }) {
  return <button className="flex min-h-9 w-full min-w-0 items-center gap-2 rounded-control px-2 text-left text-[13px] hover:bg-sunk aria-pressed:bg-sunk" aria-pressed={active} onClick={() => onSelect(r.t.id)}>
    <ThreadDot state={r.state} />
    <span className="min-w-0 flex-1 truncate" title={r.t.title}>{r.t.title}</span>
    <span className="flex-none text-[12px] text-ink-3">{r.state === "new" ? STATE_LABEL.new : `${STATE_LABEL[r.state]} · ${age(r.last)}`}</span>
  </button>;
});
