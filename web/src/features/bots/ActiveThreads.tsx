// Active threads: every bot's open threads that work, wait for you or changed in the last 24 hours, in one list over
// the bots (the phone's Chats tab, the Bots page and the laptop sidebar). A row opens the bot's chat at that thread.
// Six rows show, threads that need attention first when the cap cuts; Show all unfolds the rest in place. A
// conversation of several bots is one row with every bot's mark, opening its main bot's thread (the merged view).

import { useQueries } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { memo, useState } from "react";
import { botTone } from "@/components/BotAvatar";
import { IconButton, StatusBadge } from "@/components/ui";
import { useArchiveThread } from "@/features/chat/BotThreads";
import type { BotThread } from "@/gen/BotThread";
import { activeThreads, capThreads, type ActiveThread, type ThreadTone } from "@/lib/activeThreads";
import { api } from "@/lib/api";
import { botInitial, useBots } from "@/lib/bots";
import { age } from "@/lib/format";
import { useSessions } from "@/lib/queries";
import { isRunning, isUnseen } from "@/lib/sessionStatus";
import { useScope } from "@/lib/workspace";

const DOT: Record<ThreadTone, string> = { running: "bg-work", needs: "bg-attn", done: "bg-gh-open", failed: "bg-gh-closed", idle: "bg-line" };

/** The workspace's active threads, live: sessions come from the session store, titles from each bot's thread list
 * (the same queries as the bot page, refreshed by the `bots`/`threads` bus event). */
export function useActiveThreads(): ActiveThread[] {
  const scope = useScope();
  const bots = useBots().data;
  const sessions = useSessions().data;
  const mine = (bots ?? []).filter((b) => b.workspace_id === scope.workspace.id);
  const lists = useQueries({ queries: mine.map((b) => ({ queryKey: ["bot-threads", b.id, false], queryFn: () => api.get<BotThread[]>(`/api/bots/${b.id}/threads`), staleTime: 15_000, retry: false })) });
  // Cheap enough to compute on each render: a few bots and their sessions.
  const threads = new Map(mine.map((b, i) => [b.id, lists[i]?.data]));
  return activeThreads(mine, sessions ?? [], threads, scope.workspace.id, Date.now());
}

/** The bots of a row: "Scout", or "Scout + Docs" for a conversation of several bots. */
const botNames = (row: ActiveThread) => row.bots.map((b) => b.settings.name).join(" + ");

/** The small marks of the other bots of a conversation, after the main bot's. */
function OtherMarks({ row, size }: { row: ActiveThread; size: string }) {
  if (row.bots.length < 2) return null;
  return <span className="flex flex-none items-center gap-0.5" aria-hidden>
    {row.bots.slice(1, 4).map((b) => <span key={b.id} className={`grid ${size} place-items-center rounded-sm text-[11px] font-semibold ${botTone(b.settings.name)}`}>{botInitial(b.settings.name)}</span>)}
  </span>;
}

/** One thread in the Chats tab and on the Bots page, styled like a bot's conversation row. */
const ActiveThreadRow = memo(function ActiveThreadRow({ row, className = "" }: { row: ActiveThread; className?: string }) {
  const name = botNames(row);
  const unseen = isUnseen(row.session);
  return <Link to="/bots/$id" params={{ id: row.bot.id }} search={{ thread: row.thread } as never} className={`grid min-h-[60px] grid-cols-[36px_minmax(0,1fr)_auto] items-center gap-x-3 px-3 py-2 no-underline hover:bg-sunk ${className}`} aria-label={`${row.title}, ${name}, ${row.label}`}>
    <span className={`relative grid size-9 place-items-center rounded-card text-[15px] font-semibold ${botTone(row.bot.settings.name)}`} aria-hidden>
      {botInitial(row.bot.settings.name)}
      <span className={`absolute -right-0.5 -bottom-0.5 size-2.5 rounded-full border-2 border-paper ${DOT[row.tone]}`} />
    </span>
    <span className="grid min-w-0 gap-0.5">
      <span className={`truncate text-[13px] text-ink ${unseen || row.tone === "needs" ? "font-semibold" : "font-medium"}`} title={row.title}>{row.title}</span>
      <span className="flex min-w-0 items-center gap-1.5 text-[12px] text-ink-3"><OtherMarks row={row} size="size-4" /><span className="truncate">{name}</span></span>
    </span>
    <span className="grid justify-items-end gap-1 text-[12px] text-ink-3">
      <span>{age(row.last)}</span>
      <StatusBadge tone={row.tone} label={row.label} />
    </span>
  </Link>;
});

/** Show all (N) / Show fewer under a capped list. */
function Toggle({ all, total, onToggle, className }: { all: boolean; total: number; onToggle: () => void; className: string }) {
  return <button type="button" className={className} aria-expanded={all} onClick={onToggle}>{all ? "Show fewer" : `Show all (${total})`}</button>;
}

function useShown(rows: ActiveThread[]) {
  const [all, setAll] = useState(false);
  return { all, toggle: () => setAll(!all), shown: all ? rows : capThreads(rows) };
}

/** The section over the bots of the Chats tab (`list`: rows with borders) and of the Bots page (`cards`). Nothing
 * when no thread is active. */
export function ActiveThreadsSection({ variant, labelClass }: { variant: "list" | "cards"; labelClass: string }) {
  const rows = useActiveThreads();
  const { all, toggle, shown } = useShown(rows);
  if (!rows.length) return null;
  const capped = rows.length > shown.length || all;
  const row = variant === "list" ? "border-b border-line-2" : "rounded-card border border-line bg-paper";
  return <section className={variant === "cards" ? "grid grid-cols-[minmax(0,1fr)] gap-2" : ""} aria-label="Active threads" data-tour="bots.active-threads">
    <h2 className={labelClass}>Active threads</h2>
    {shown.map((r) => <ActiveThreadRow key={r.conversation} row={r} className={row} />)}
    {capped && <Toggle all={all} total={rows.length} onToggle={toggle} className={variant === "list" ? "flex min-h-11 w-full items-center px-3 text-[13px] text-teal-ink" : "justify-self-start text-[12px] text-teal-ink hover:underline"} />}
  </section>;
}

/** A sidebar row: the link to the thread, then a small Archive (one click, like an agent's), shown on hover and
 * keyboard focus. Disabled while the thread works. */
const SidebarThreadRow = memo(function SidebarThreadRow({ r, archiving, onArchive }: { r: ActiveThread; archiving: boolean; onArchive: () => void }) {
  const running = isRunning(r.session);
  return <div className="group flex items-start gap-0.5 rounded-control hover:bg-sunk has-[.active]:bg-paper has-[.active]:shadow-[inset_0_0_0_1px_var(--color-line)]">
    <Link to="/bots/$id" params={{ id: r.bot.id }} search={{ thread: r.thread } as never} className="flex min-w-0 flex-1 items-start gap-2 rounded-control py-1 pl-[9px] text-ink no-underline" title={`${r.title} · ${botNames(r)} · ${r.label}`} aria-label={`${r.title}, ${botNames(r)}, ${r.label}`}>
      <span className={`mt-0.5 grid size-[16px] flex-none place-items-center rounded-sm text-[11px] font-semibold ${botTone(r.bot.settings.name)}`} aria-hidden>{botInitial(r.bot.settings.name)}</span>
      <span className="grid min-w-0 flex-1">
        <span className={`truncate text-[13px] ${isUnseen(r.session) || r.tone === "needs" ? "font-semibold" : ""}`}>{r.title}</span>
        <span className="flex min-w-0 items-center gap-1 text-[11px] text-ink-3">
          {r.tone === "running" ? <span className="lp-spin" aria-hidden /> : <span className={`size-[7px] flex-none rounded-full ${DOT[r.tone]}`} aria-hidden />}
          <span className="flex-none">{r.label} · {age(r.last)}</span><span className="truncate">· {botNames(r)}</span>
        </span>
      </span>
    </Link>
    <span className="lp-row-more flex-none self-center pr-0.5">
      <IconButton icon="archive" label={running ? `Stop its turn first to archive “${r.title}”` : `Archive “${r.title}”`} disabled={running || archiving} onClick={onArchive} />
    </span>
  </div>;
});

/** The laptop sidebar's part over the Bots group: two lines per thread (its title, then a state mark, the status,
 * the time and the bot). Hidden while the sidebar shows icons only or no thread is active. */
export function SidebarActiveThreads() {
  const rows = useActiveThreads();
  const { all, toggle, shown } = useShown(rows);
  const { archive, busy } = useArchiveThread();
  if (!rows.length) return null;
  return <div className="mb-2 flex flex-none flex-col gap-px border-b border-line-2 pb-2" data-tour="sidebar.active-threads">
    <div className="px-[9px] pb-0.5 text-[11px] font-semibold text-ink-3">Active threads</div>
    {shown.map((r) => <SidebarThreadRow key={r.conversation} r={r} archiving={busy(r.bot.id, r.thread)} onArchive={() => void archive(r.bot.id, r.thread, r.conversation)} />)}
    {(rows.length > shown.length || all) && <Toggle all={all} total={rows.length} onToggle={toggle} className="px-[9px] py-0.5 text-left text-[12px] text-teal-ink hover:underline" />}
  </div>;
}
