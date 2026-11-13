// A bot as a conversation (the Bots list and the phone's Chats tab): its state dot, the last message of its thread
// and its time, bold while unseen, with an amber count when it needs you. BOT.md's first line shows only for a bot
// that never ran.

import { Link } from "@tanstack/react-router";
import { memo } from "react";
import { Count } from "@/components/ui";
import type { Bot } from "@/gen/Bot";
import type { Session } from "@/gen/Session";
import { botInitial, botJob, botState, threadSummary, type BotState } from "@/lib/bots";
import { age } from "@/lib/format";
import { isUnseen } from "@/lib/sessionStatus";
import { isNoUpdate } from "@/lib/routines";

const DOT: Record<BotState, string> = { needs: "bg-attn", running: "bg-work", paused: "bg-attn", idle: "bg-line", new: "bg-line" };
const STATE_LABEL: Record<BotState, string> = { needs: "Needs you", running: "Working", paused: "Quota paused", idle: "Idle", new: "No task yet" };

/** The newest session of the thread that said something, else the newest one. */
function lastTurn(thread: Session[]): Session | undefined {
  return [...thread].reverse().find((s) => s.last_message && !isNoUpdate(s.last_message)) ?? thread.at(-1);
}

export const BotConversationRow = memo(function BotConversationRow({ bot, thread, pinned, className = "" }: { bot: Bot; thread: Session[]; pinned?: boolean; className?: string }) {
  const state = botState(thread);
  const last = lastTurn(thread);
  const newest = thread.at(-1);
  const unseen = !!newest && isUnseen(newest) && newest.status !== "closed";
  // Over all its threads: how many wait for you, and whether one works.
  const { needs: waiting, working: workingThreads } = threadSummary(thread);
  const working = workingThreads > 0;
  const line = working ? `Working${workingThreads > 1 ? ` on ${workingThreads} threads` : ""}${last?.last_message ? ` · ${last.last_message}` : "…"}` : last?.last_message ?? (thread.length ? "No message yet" : botJob(bot) || "No instructions yet");
  return <Link to="/bots/$id" params={{ id: bot.id }} className={`grid min-h-[60px] grid-cols-[36px_minmax(0,1fr)_auto] items-center gap-x-3 px-3 py-2 no-underline hover:bg-sunk ${className}`} aria-label={`${bot.settings.name}, ${STATE_LABEL[state]}`}>
    <span className="relative grid size-9 place-items-center rounded-card bg-teal-soft text-[15px] font-semibold text-teal-ink" aria-hidden>
      {botInitial(bot.settings.name)}
      <span className={`absolute -right-0.5 -bottom-0.5 size-2.5 rounded-full border-2 border-paper ${DOT[state]}`} title={STATE_LABEL[state]} />
    </span>
    <span className="grid min-w-0 gap-0.5">
      <span className={`flex min-w-0 items-center gap-1.5 text-[13px] text-ink ${unseen || waiting ? "font-semibold" : "font-medium"}`}>
        <span className="truncate">{bot.settings.name}</span>{pinned && <span className="shrink-0 text-[11px] font-normal text-ink-3" title="Pinned: your Ask bot">Ask</span>}
      </span>
      <span className={`truncate text-[12px] ${working ? "text-work" : unseen ? "font-semibold text-ink-2" : "text-ink-3"}`} title={line}>{line}</span>
    </span>
    <span className="grid justify-items-end gap-1 text-[12px] text-ink-3">
      <span>{newest ? age(newest.status_since) : ""}</span>
      <Count n={waiting} needs />
    </span>
  </Link>;
});
