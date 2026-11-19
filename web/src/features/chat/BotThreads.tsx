// A bot's threads: its parallel conversations, each one session (the owner's threads, one per GitHub item,
// channel, hook, agent or Telegram, one per routine run). The bot page shows one thread at a time (`?thread=`, the
// only place that state lives); switching replaces the history entry, like tabs. On the laptop a bar under the
// header: New thread first (one click), then the most recent threads, each with its state dot, then All threads
// with a search, and a quiet Archive of the thread shown at the end; on the phone a New thread button, a sheet and an
// Archive button (with its confirmation in a popover) in the chat's bar. Rename and Archive are also in the page's ⋯.
// A thread in another bot's conversation says so ("in Scout's conversation") and opens the merged view.

import { useQueryClient } from "@tanstack/react-query";
import { memo, useMemo, useState, type ReactNode } from "react";
import type { Bot } from "@/gen/Bot";
import type { BotThread } from "@/gen/BotThread";
import type { Session } from "@/gen/Session";
import { ActionButton } from "@/components/ActionButton";
import { Icon } from "@/components/Icon";
import { Popover } from "@/components/Popover";
import { toastError } from "@/components/Toast";
import { ConfirmButton, FilterChip, FilterChips, IconButton } from "@/components/ui";
import { actionConfirm, runAction } from "@/lib/actions";
import { botThread, threadState, useBotThreads, useBots, type BotState } from "@/lib/bots";
import { mainBotOf } from "@/lib/conversations";
import { age } from "@/lib/format";
import { usePhone } from "@/lib/phone";
import { isRunning } from "@/lib/sessionStatus";

/** The `?thread=` value of a thread the owner starts with their next message. */
export const NEW_THREAD = "new";

const DOT: Record<BotState, string> = { needs: "bg-attn", running: "bg-work", paused: "bg-attn", idle: "bg-line", new: "bg-line" };
const STATE_LABEL: Record<BotState, string> = { needs: "Needs you", running: "Working", paused: "Quota paused", idle: "Idle", new: "No message yet" };

/** A thread with its state and last activity, from the bot's sessions. */
export type ThreadRow = { t: BotThread; state: BotState; last: number };

/** The threads of a bot with their states, by last activity, newest first. */
export function useThreadRows(bot: Bot, sessions: Session[]): ThreadRow[] {
  const threads = useBotThreads(bot.id).data;
  return useMemo(() => {
    const rows = (threads ?? []).map((t): ThreadRow => {
      const mine = botThread(sessions, bot.id, t.id);
      return { t, state: threadState(mine), last: mine.at(-1)?.status_since ?? t.created_at };
    });
    return rows.sort((a, b) => b.last - a.last);
  }, [threads, sessions, bot.id]);
}

/** "in Scout's conversation" for a thread this bot joined in another bot's conversation, else null. */
export function useJoinedLabel(): (t: BotThread) => string | null {
  const bots = useBots().data;
  return (t) => {
    const main = mainBotOf(t.conversation);
    if (main === t.bot_id) return null;
    return `in ${bots?.find((b) => b.id === main)?.settings.name ?? "another bot"}'s conversation`;
  };
}

export const ThreadDot = ({ state }: { state: BotState }) => <span className={`size-2 flex-none rounded-full ${DOT[state]}`} title={STATE_LABEL[state]} aria-hidden />;

/** Chips shown in the laptop bar besides the selected thread. */
const RECENT = 5;

/** The laptop's thread bar under the header: New thread (one click), the most recent threads, then All threads. */
export function ThreadBar({ bot, rows, selected, onSelect, archive }: { bot: Bot; rows: ThreadRow[]; selected: string; onSelect: (thread: string) => void; archive?: ReactNode }) {
  const shown = useMemo(() => {
    const recent = rows.slice(0, RECENT);
    const sel = rows.find((r) => r.t.id === selected);
    return [...recent, ...(sel && !recent.includes(sel) ? [sel] : [])];
  }, [rows, selected]);
  const joined = useJoinedLabel();
  return <div className="flex min-h-10 flex-none items-center gap-1.5 border-b border-line-2 px-5 py-1.5" data-tour="bots.threads">
    {/* Shown pressed while the new thread is the one shown (its first message starts it). */}
    <button className={`btn btn-sm flex-none ${selected === NEW_THREAD ? "border-teal bg-teal-soft text-teal-ink" : ""}`} aria-pressed={selected === NEW_THREAD} title="Start a new thread: your next message starts it, next to the others" onClick={() => onSelect(NEW_THREAD)} data-tour="bots.new-thread"><Icon name="plus" size={12} />New thread</button>
    <FilterChips label={`Threads of ${bot.settings.name}`} className="min-w-0 flex-1 flex-nowrap! overflow-hidden">
      {shown.map((r) => <FilterChip key={r.t.id} active={r.t.id === selected} title={`${r.t.title}${joined(r.t) ? ` · ${joined(r.t)}` : ""} · ${STATE_LABEL[r.state]}`} onClick={() => onSelect(r.t.id)}>
        <ThreadDot state={r.state} /><span className="max-w-[180px] truncate">{r.t.title}</span>
      </FilterChip>)}
    </FilterChips>
    <Popover align="right" width={340} viewport label="All threads" trigger={({ open, toggle }) => <button className="btn btn-sm flex-none" aria-expanded={open} aria-haspopup="dialog" onClick={toggle}>All threads ({rows.length})</button>}>
      {(close) => <ThreadList bot={bot} rows={rows} selected={selected} onSelect={(t) => { close(); onSelect(t); }} />}
    </Popover>
    {archive}
  </div>;
}

/** Every thread with a search (the laptop's All threads, the phone's Threads sheet, where the search is not focused
 * so the keyboard stays closed), New thread first, then the archived threads behind a toggle, each with Bring back. The grids
 * use one `minmax(0, 1fr)` column (`grid-cols-1`): an implicit auto column grows to the full width of a truncated title. */
export function ThreadList({ bot, rows, selected, onSelect }: { bot: Bot; rows: ThreadRow[]; selected: string; onSelect: (thread: string) => void }) {
  const phone = usePhone();
  const [query, setQuery] = useState("");
  const [archived, setArchived] = useState(false);
  const old = useBotThreads(archived ? bot.id : null, true).data?.filter((t) => t.archived_at != null) ?? [];
  const q = query.trim().toLowerCase();
  const list = q ? rows.filter((r) => r.t.title.toLowerCase().includes(q)) : rows;
  const qc = useQueryClient();
  const joined = useJoinedLabel();
  return <div className="grid grid-cols-1 gap-1 p-1.5">
    {rows.length > 6 && <input className="field-input" placeholder="Search threads" aria-label="Search threads" value={query} onChange={(e) => setQuery(e.target.value)} autoFocus={!phone} />}
    <button className="flex min-h-9 items-center gap-2 rounded-control px-2 text-left text-[13px] hover:bg-sunk" aria-pressed={selected === NEW_THREAD} onClick={() => onSelect(NEW_THREAD)}>
      <Icon name="plus" size={12} className="flex-none text-ink-3" /><span className="min-w-0 flex-1 truncate">New thread</span><span className="flex-none text-[12px] text-ink-3">your next message starts it</span>
    </button>
    <div className="grid max-h-[50vh] grid-cols-1 gap-px overflow-y-auto">
      {list.map((r) => <ThreadItem key={r.t.id} r={r} joined={joined(r.t)} active={r.t.id === selected} onSelect={onSelect} />)}
      {list.length === 0 && <p className="m-0 px-2 py-1.5 text-[12px] text-ink-3">No thread matches.</p>}
    </div>
    <button className="chat-tap justify-self-start px-2 text-[12px] text-teal-ink hover:underline" aria-expanded={archived} onClick={() => setArchived(!archived)}>{archived ? "Hide archived threads" : "Show archived threads"}</button>
    {archived && <div className="grid grid-cols-1 gap-px">
      {old.length === 0 ? <p className="m-0 px-2 py-1 text-[12px] text-ink-3">No archived thread.</p>
        : old.map((t) => <div key={t.id} className="flex min-h-9 items-center gap-2 px-2 text-[13px] text-ink-3">
          <Icon name="archive" size={12} className="flex-none" /><span className="min-w-0 flex-1 truncate" title={t.title}>{t.title}</span>
          <ActionButton name="bot.thread-archive" params={{ id: bot.id, thread: t.id }} body={{ archived: false }} consequence="Brings the thread and its session back." confirmLabel="Bring back" icon={null} onDone={() => qc.invalidateQueries({ queryKey: ["bot-threads"] })}>Bring back</ActionButton>
        </div>)}
    </div>}
  </div>;
}

const ThreadItem = memo(function ThreadItem({ r, joined, active, onSelect }: { r: ThreadRow; joined: string | null; active: boolean; onSelect: (thread: string) => void }) {
  return <button className="flex min-h-9 w-full min-w-0 items-center gap-2 rounded-control px-2 text-left text-[13px] hover:bg-sunk aria-pressed:bg-sunk" aria-pressed={active} onClick={() => onSelect(r.t.id)}>
    <ThreadDot state={r.state} />
    <span className="grid min-w-0 flex-1"><span className="truncate" title={r.t.title}>{r.t.title}</span>{joined && <span className="truncate text-[11px] text-ink-3">{joined}</span>}</span>
    <span className="flex-none text-[12px] text-ink-3">{r.state === "new" ? STATE_LABEL.new : `${STATE_LABEL[r.state]} · ${age(r.last)}`}</span>
  </button>;
});

/** Archive of the thread shown, in the chat itself (the same action as ⋯ → Archive this thread): on the laptop a quiet
 * button at the end of the thread bar that asks inline (ConfirmButton); on the phone an icon of the chat's bar whose
 * popover asks (it closes first: closing pops its history entry, so the page's switch to another thread stays). Disabled while the thread works. `onDone` runs after the lists refreshed (the page then shows another
 * thread). */
export function ArchiveThreadButton({ bot, thread, sessions, onDone }: { bot: Bot; thread: BotThread; sessions: Session[]; onDone: () => void }) {
  const phone = usePhone();
  const qc = useQueryClient();
  const [busy, setBusy] = useState(false);
  const running = sessions.some(isRunning);
  const params = { id: bot.id, thread: thread.id };
  const consequence = actionConfirm("bot.thread-archive", params) ?? "Archives this thread and its open session.";
  const archive = async () => {
    setBusy(true);
    try {
      await runAction("bot.thread-archive", params, { archived: true });
      await Promise.all([qc.invalidateQueries({ queryKey: ["bot-threads"] }), qc.invalidateQueries({ queryKey: ["sessions"] })]);
      onDone();
    } catch (e) { toastError(e); }
    finally { setBusy(false); }
  };
  const title = running ? "Stop its turn first to archive this thread" : `Archive “${thread.title}”: ${consequence}`;
  if (!phone)
    return <span className="flex flex-none items-center" data-tour="bots.archive-thread">
      <ConfirmButton className="btn-ghost text-ink-3" icon="archive" disabled={running || busy} title={title} consequence="Archive this thread?" confirmLabel="Archive" onConfirm={archive}>{busy ? "Archiving…" : "Archive"}</ConfirmButton>
    </span>;
  return <Popover align="right" width={300} viewport label="Archive this thread" trigger={({ open, toggle }) => <IconButton icon="archive" label={running ? title : "Archive this thread"} disabled={running || busy} aria-expanded={open} aria-haspopup="dialog" data-tour="bots.archive-thread" onClick={toggle} />}>
    {(close) => <div className="grid gap-2 p-3" role="group" aria-live="polite">
      <p className="m-0 text-[13px] font-semibold">Archive “{thread.title}”?</p>
      <p className="m-0 text-[13px] text-ink-2">{consequence}</p>
      <div className="flex justify-end gap-2">
        <button type="button" className="btn btn-sm btn-ghost" onClick={close}>Cancel</button>
        <button type="button" className="btn btn-sm btn-danger" autoFocus onClick={() => { close(); void archive(); }}><Icon name="archive" size={12} />Archive</button>
      </div>
    </div>}
  </Popover>;
}
