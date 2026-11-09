// The one field of the top bar: Ask (a message to a bot of the workspace, shown in the Ask panel) and Launch (an agent
// at once, with the page as context and the folder's defaults; its menu opens the full composer with the text).
// The page you are on is the context: a folder, a GitHub item, a session's trace, a Local PR or a settings page.
// Ask is the primary button (and Enter) on every page; Launch is an outline button. Without a bot, Launch is the
// only button, so it is the primary one.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useRouterState } from "@tanstack/react-router";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { Bot } from "@/gen/Bot";
import type { Predraft } from "@/gen/Predraft";
import type { Project } from "@/gen/Project";
import type { Session } from "@/gen/Session";
import { Icon } from "@/components/Icon";
import { toast, toastError } from "@/components/Toast";
import { LaunchSplitButton, openOptions } from "@/features/launch/LaunchButton";
import { itemKey } from "@/features/github/lib";
import type { Item } from "@/features/github/types";
import { runAction } from "@/lib/actions";
import { askTarget, useBots } from "@/lib/bots";
import { SETTINGS_NAV, WORKSPACE_NAV } from "@/features/settings/nav";
import { useProjects, useSessions } from "@/lib/queries";
import { useScope } from "@/lib/workspace";
import { useAskPanel } from "@/stores/ask";
import { useComposer, type LaunchContext } from "@/stores/composer";

type PageContext = {
  /** Shown in the field; null: the page adds nothing (Today, lists). */
  label: string | null;
  /** The line added to an Ask message so the bot knows what you look at. */
  line: string | null;
  launch?: LaunchContext;
  /** On a bot's page, Ask talks to that bot. */
  botId?: string;
};

function pageContext(path: string, search: Record<string, unknown>, projects: Project[], sessions: Session[], bots: Bot[], item: Item | undefined, predraft: Predraft | undefined): PageContext {
  const project = path.match(/^\/projects\/([^/]+)/);
  if (project) {
    const p = projects.find((x) => x.id === project[1]);
    return { label: p ? `Folder ${p.name}` : null, line: p ? `the folder ${p.name} (folder id ${p.id})` : null, launch: p ? { projectId: p.id } : undefined };
  }
  const gh = path.match(/^\/github\/([^/]+)\/([^/]+)\/(\d+)/);
  if (gh) {
    const repo = `${gh[1]}/${gh[2]}`;
    const number = Number(gh[3]);
    const pr = item?.__typename === "PullRequest";
    const lower = repo.toLowerCase();
    const p = projects.find((x) => x.gh_repo?.toLowerCase() === lower) ?? projects.find((x) => x.gh_upstream?.toLowerCase() === lower);
    const url = `https://github.com/${repo}/${pr ? "pull" : "issues"}/${number}`;
    return {
      label: `${repo}#${number}`, line: `${pr ? "the pull request" : "the GitHub item"} ${repo}#${number}${item ? ` “${item.title}”` : ""} (${url})`,
      launch: { projectId: p?.id ?? null, githubRefs: [{ url, kind: pr ? "pr" : "issue", repo, number, title: item?.title ?? null }] },
    };
  }
  const session = path.startsWith("/agents") && typeof search.session === "string" ? sessions.find((s) => s.id === search.session) : undefined;
  if (session) {
    return {
      label: `Agent ${session.title}`, line: `the agent session “${session.title}” (session id ${session.id})`,
      launch: { projectId: session.project_id && session.launch_spec.target.kind !== "temp" ? session.project_id : undefined, agentTraces: session.last_seq > 0 ? [{ session_id: session.id, through_seq: session.last_seq }] : [] },
    };
  }
  const local = path.match(/^\/predrafts\/([^/]+)/);
  if (local) return { label: predraft ? `Local PR ${predraft.title}` : "Local PR", line: `the Local PR ${predraft ? `“${predraft.title}” ` : ""}(id ${local[1]})`, launch: predraft ? { projectId: predraft.project_id } : undefined };
  const settings = path.match(/^\/settings\/?([^/]*)/);
  if (settings) {
    const page = [...WORKSPACE_NAV, ...SETTINGS_NAV.flatMap((g) => g.items)].find((e) => e.to === path.replace(/\/$/, ""))?.label ?? (settings[1] || "Appearance");
    return { label: `Settings · ${page}`, line: `the settings page ${page} (${path})` };
  }
  const bot = path.match(/^\/bots\/([^/]+)/);
  if (bot && bots.some((b) => b.id === bot[1])) return { label: null, line: null, botId: bot[1] };
  return { label: null, line: null };
}

export function CommandField() {
  const qc = useQueryClient();
  const path = useRouterState({ select: (s) => s.location.pathname });
  const search = useRouterState({ select: (s) => s.location.search as Record<string, unknown> });
  const projects = useProjects().data ?? [];
  const sessions = useSessions().data ?? [];
  const scope = useScope();
  const allBots = useBots().data ?? [];
  const bots = allBots.filter((b) => b.workspace_id === scope.workspace.id);
  const gh = path.match(/^\/github\/([^/]+)\/([^/]+)\/(\d+)/);
  // The item and Local PR the page already cached; the field never fetches them.
  const item = useQuery<Item>({ queryKey: gh ? itemKey(`${gh[1]}/${gh[2]}`, Number(gh[3])) : ["none"], enabled: false }).data;
  const predraftId = path.match(/^\/predrafts\/([^/]+)/)?.[1];
  const predraft = useQuery<{ predraft: Predraft }>({ queryKey: ["predrafts", predraftId ?? ""], enabled: false }).data?.predraft;
  const page = pageContext(path, search, projects, sessions, allBots, item, predraft);
  const [withContext, setWithContext] = useState(true);
  const pageKey = `${path}:${String(search.session ?? "")}`;
  useEffect(() => setWithContext(true), [pageKey]);
  const ctx = withContext ? page : { label: null, line: null, launch: undefined, botId: page.botId };
  const [picked, setPicked] = useState<string | null>(null);
  const bot = bots.find((b) => b.id === (ctx.botId ?? picked)) ?? askTarget(allBots, scope.workspace.id);
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const ref = useRef<HTMLTextAreaElement>(null);
  useLayoutEffect(() => { const el = ref.current; if (!el) return; el.style.height = "auto"; el.style.height = `${Math.min(el.scrollHeight, 96)}px`; }, [text]);
  const panel = useAskPanel();

  const ask = async () => {
    if (!bot || busy) return;
    const t = text.trim();
    // An empty Ask opens the chat.
    if (!t) { if (!ctx.botId) panel.show(bot.id); return; }
    setBusy(true);
    try {
      await runAction("bot.message", { id: bot.id }, { text: ctx.line ? `${t}\n\n(I am looking at ${ctx.line}.)` : t });
      setText("");
      await qc.invalidateQueries({ queryKey: ["sessions"] });
      // On the bot's own page, its chat is already there.
      if (!ctx.botId) panel.show(bot.id);
    } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  const launch = () => {
    const c = useComposer.getState();
    const t = text.trim();
    // A kept draft, or nothing to launch: open the composer instead.
    if (!t || c.open || c.spec.text.trim()) { openOptions(ctx.launch, t); setText(""); return; }
    c.launchNow(ctx.launch, t);
    setText("");
    // Still not launched (no profile, no folder, a busy worktree): show the composer and what it needs.
    setTimeout(() => {
      if (!useComposer.getState().autoLaunchUntil) return;
      useComposer.setState({ autoLaunchUntil: 0 });
      useComposer.getState().setMinimized(false);
      toast("Finish the launch options, then Launch.");
    }, 4_200);
  };
  // Ask is the primary button (and Enter) on every page; without a bot in the workspace, Launch is.
  const primary = bot ? () => void ask() : launch;
  const askLabel = bot ? `Ask ${bot.settings.name}` : "Ask";
  return (
    <div className="flex min-w-[280px] max-w-[760px] flex-1 items-center gap-1.5" data-tour="topbar.field">
      <div className="flex min-w-0 flex-1 items-center gap-1.5 overflow-hidden rounded-lg border border-line bg-paper px-2 py-1 focus-within:border-teal">
        <Icon name="spark" size={14} className="flex-none text-teal-ink" />
        {!ctx.botId && bots.length > 1 && bot && <select className="min-w-0 max-w-[120px] shrink bg-transparent text-[12px] font-semibold outline-none" aria-label="Bot that Ask talks to" title="Ask sends to this bot" value={bot.id} onChange={(e) => setPicked(e.target.value)}>{bots.map((b) => <option key={b.id} value={b.id}>{b.settings.name}</option>)}</select>}
        {ctx.label && <span className="flex min-w-[28px] max-w-[180px] shrink items-center gap-1 rounded bg-sunk px-1.5 text-[11px] text-ink-2" title="The page you are on goes with Ask and Launch">
          <span className="truncate">{ctx.label}</span>
          <button className="flex-none text-ink-3 hover:text-ink" aria-label="Leave out the page" title="Leave out the page" onClick={() => setWithContext(false)}><Icon name="x" size={10} /></button>
        </span>}
        <textarea ref={ref} rows={1} value={text} disabled={busy} aria-label={bot ? "Message to a bot, or prompt for a new agent" : "Prompt for a new agent"}
          placeholder={bot ? "Ask, or launch an agent…" : "Launch an agent…"}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) { e.preventDefault(); primary(); } }}
          className="min-h-6 min-w-[48px] flex-1 resize-none bg-transparent placeholder-shown:overflow-hidden placeholder-shown:whitespace-nowrap py-0.5 text-[13px] leading-5 outline-none" />
      </div>
      {bot && <button className="btn btn-primary" data-tour="topbar.ask" title={`${askLabel}: the answer opens in the Ask panel${ctx.line ? ", with the page as context" : ""}`} disabled={busy} onClick={() => void ask()}><Icon name="send" size={13} />Ask</button>}
      <LaunchSplitButton ctx={ctx.launch} text={text} onLaunch={launch} onUsed={() => setText("")} primary={!bot}
        title={text.trim() ? `Launch an agent at once${ctx.launch ? " with the page as context" : ""}, with the folder's defaults` : "Open the composer"} />
    </div>
  );
}
