// The Ask panel: the chat of a bot (the same BotChat as its page) beside the current page, opened by Ask in the top
// bar. It shows the bot the field sent to, else the workspace's default bot (`askTarget`); a deleted bot falls back
// to the default. The thread is the one the field's message went to, else the one the owner looked at last
// (`landingThread`); New thread starts another with the next message. No code of its own for Ask Launchpad: any bot
// works here.

import { Link, useNavigate } from "@tanstack/react-router";
import { useMemo } from "react";
import { Icon } from "@/components/Icon";
import { askTarget, botProfile, botThread, findThread, landingThread, rememberThread, rotationReason, useBots, useBotThreads } from "@/lib/bots";
import { useProfiles } from "@/lib/profiles";
import { useSessions } from "@/lib/queries";
import { useScope } from "@/lib/workspace";
import { useAskPanel } from "@/stores/ask";
import { BotChat } from "./BotChat";
import { NEW_THREAD } from "./BotThreads";

export function AskPanel() {
  const { botId, close, show, thread: asked, setThread } = useAskPanel();
  const scope = useScope();
  const all = useBots().data ?? [];
  const bots = all.filter((b) => b.workspace_id === scope.workspace.id);
  const bot = bots.find((b) => b.id === botId) ?? askTarget(all, scope.workspace.id);
  const sessions = useSessions().data;
  const threads = useBotThreads(bot?.id).data;
  const threadId = asked === NEW_THREAD ? NEW_THREAD : findThread(threads, asked ?? undefined)?.id ?? asked ?? (bot && threads ? landingThread(bot.id, threads, sessions ?? []) : null) ?? NEW_THREAD;
  const title = threadId === NEW_THREAD ? "New thread" : threads?.find((t) => t.id === threadId)?.title ?? null;
  const thread = useMemo(() => (bot && threadId !== NEW_THREAD ? botThread(sessions ?? [], bot.id, threadId) : []), [sessions, bot, threadId]);
  const profiles = useProfiles();
  const navigate = useNavigate();
  const profile = bot ? botProfile(bot, profiles) : null;
  return (
    <aside aria-label="Ask" data-tour="ask.panel" className="flex w-[min(440px,42vw)] min-w-[320px] flex-none flex-col border-l border-line bg-paper">
      <div className="flex min-h-11 items-center gap-1.5 border-b border-line px-2.5">
        <Icon name="spark" size={14} className="flex-none text-teal-ink" />
        {bots.length > 1 && bot
          ? <select className="min-w-0 flex-1 truncate bg-transparent text-[13px] font-semibold outline-none" aria-label="Bot" value={bot.id} onChange={(e) => show(e.target.value)}>{bots.map((b) => <option key={b.id} value={b.id}>{b.settings.name}</option>)}</select>
          : <b className="min-w-0 flex-1 truncate text-[13px]">{bot?.settings.name ?? "Ask"}</b>}
        {bot && title && <span className="max-w-[40%] min-w-0 truncate text-[12px] text-ink-3" title={title}>{title}</span>}
        {bot && <button className="iconbtn" title="New thread: your next message starts it" aria-label="New thread" disabled={threadId === NEW_THREAD} onClick={() => setThread(NEW_THREAD)}><Icon name="plus" size={14} /></button>}
        {bot && <Link to="/bots/$id" params={{ id: bot.id }} search={(threadId !== NEW_THREAD ? { thread: threadId } : {}) as never} className="iconbtn" title={`Open ${bot.settings.name}'s page`} aria-label="Open the bot's page"><Icon name="ext" size={14} /></Link>}
        <button className="iconbtn" title="Close the panel" aria-label="Close the Ask panel" onClick={close}><Icon name="x" size={14} /></button>
      </div>
      {bot
        ? <BotChat key={`${bot.id}/${threadId}`} view="ask" bot={bot} threadId={threadId} thread={thread} rotation={rotationReason(bot, thread.at(-1) ?? null, profile)} needsProfile={!profile}
            onThread={(t) => { if (t !== threadId) { rememberThread(bot.id, t); setThread(t); } }}
            openSettings={() => void navigate({ to: "/bots/$id", params: { id: bot.id }, search: { panel: "about" } as never })}
            intro={<p className="m-0 text-[13px] text-ink-2">Ask {bot.settings.name} about your agents, folders and Local PRs, or to change Launchpad for you. Its answers end in cards you start with one click; each change it makes shows as a line that links to what changed.</p>} />
        : <div className="grid flex-1 place-items-center p-6 text-center text-[13px] text-ink-2"><div>This workspace has no bot. <Link to="/bots" className="underline">Create one</Link> to ask it things.</div></div>}
    </aside>
  );
}
