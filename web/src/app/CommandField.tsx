// The one field of the top bar: Ask (a message to a bot of the workspace, shown in the Ask panel) and Launch (an agent
// at once, with the page as context and the folder's defaults; its menu opens the full composer with the text).
// The page you are on is the context: a folder, a session's trace, a Local PR or a settings page.
// Pasted screenshots go with the message (Ask) or the launch, as small thumbnails in the field; no file picker.
// Ask is the primary button (and Enter) on every page, labeled with the bot it sends to; its arrow picks another bot
// of the workspace (`askRecipient` decides the bot for both). Launch is an outline button. Without a bot, Launch is
// the only button, so it is the primary one.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate, useRouterState } from "@tanstack/react-router";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { Bot } from "@/gen/Bot";
import type { BotSend } from "@/gen/BotSend";
import type { Predraft } from "@/gen/Predraft";
import type { Project } from "@/gen/Project";
import type { Session } from "@/gen/Session";
import { Icon } from "@/components/Icon";
import { BotAvatar } from "@/components/BotAvatar";
import { MenuItem, Popover } from "@/components/Popover";
import { askRecipient, NO_PICKS, pickRecipient, type AskPicks } from "@/lib/askRecipient";
import { toast, toastError } from "@/components/Toast";
import { LaunchSplitButton, openOptions } from "@/features/launch/LaunchButton";
import { runAction } from "@/lib/actions";
import { askTarget, showAccepted, useBots } from "@/lib/bots";
import { attachmentSrc, pastedImages } from "@/lib/attachments";
import { ImageThumb } from "@/components/ImageViewer";
import { ATTACH_LIMITS, useMessageAttachments } from "@/features/agents/MessageAttachments";
import { SETTINGS_NAV, WORKSPACE_NAV } from "@/features/settings/nav";
import { useProjects, useSessions } from "@/lib/queries";
import { useScope } from "@/lib/workspace";
import { useAskPanel } from "@/stores/ask";
import { useLayout } from "@/stores/layout";
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

function pageContext(path: string, sessionId: string | null, projects: Project[], sessions: Session[], bots: Bot[], predraft: Predraft | undefined): PageContext {
  const project = path.match(/^\/projects\/([^/]+)/);
  if (project) {
    const p = projects.find((x) => x.id === project[1]);
    return { label: p ? `Folder ${p.name}` : null, line: p ? `the folder ${p.name} (folder id ${p.id})` : null, launch: p ? { projectId: p.id } : undefined };
  }
  const session = sessionId ? sessions.find((s) => s.id === sessionId) : undefined;
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
  // The Local PR the page already cached; the field never fetches it.
  const predraftId = path.match(/^\/predrafts\/([^/]+)/)?.[1];
  const predraft = useQuery<{ predraft: Predraft }>({ queryKey: ["predrafts", predraftId ?? ""], enabled: false }).data?.predraft;
  // The agent shown in Agents: the address on the phone (and from links), the selected session on a laptop, which
  // clears the address once it opened the session.
  const selectedSession = useLayout((s) => s.selectedSession);
  const sessionId = path.startsWith("/agents") ? (typeof search.session === "string" ? search.session : selectedSession) : null;
  const page = pageContext(path, sessionId, projects, sessions, allBots, predraft);
  const [withContext, setWithContext] = useState(true);
  const pageKey = `${path}:${sessionId ?? ""}`;
  useEffect(() => setWithContext(true), [pageKey]);
  const ctx = withContext ? page : { label: null, line: null, launch: undefined, botId: page.botId };
  const [picks, setPicks] = useState<AskPicks>(NO_PICKS);
  const workspaceId = scope.workspace.id;
  const bot = askRecipient(bots, askTarget(allBots, workspaceId), workspaceId, pageKey, ctx.botId, picks);
  const pick = (id: string) => setPicks((p) => pickRecipient(p, workspaceId, pageKey, ctx.botId, id));
  // On the bot's own page, Ask talks in the chat it shows; to another bot, through the Ask panel.
  const own = !!bot && ctx.botId === bot.id;
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const ref = useRef<HTMLTextAreaElement>(null);
  useLayoutEffect(() => { const el = ref.current; if (!el) return; el.style.height = "auto"; el.style.height = `${Math.min(el.scrollHeight, 96)}px`; }, [text]);
  const panel = useAskPanel();
  const navigate = useNavigate();
  const shots = useMessageAttachments(true);
  const { files, reading } = shots;

  const ask = async () => {
    if (!bot || busy) return;
    const t = text.trim();
    // An empty Ask opens the chat (the thread the owner looked at last).
    if (!t && !files.length) { if (!own) panel.show(bot.id); return; }
    if (reading) return;
    setBusy(true);
    try {
      // On the bot's own page: the thread it shows; elsewhere a new thread.
      const shown = own && typeof search.thread === "string" ? search.thread : null;
      const where = shown && shown !== "new" && !shown.includes(":") ? { thread: shown } : {};
      // Home stores the message and answers at once (`accept`): the chat shows it while the bot starts.
      const sent = await runAction<BotSend>("bot.message", { id: bot.id }, { text: ctx.line ? `${t}\n\n(I am looking at ${ctx.line}.)` : t, attachments: files, ...where, accept: true });
      setText("");
      shots.clear();
      showAccepted(qc, sent);
      const thread = sent.thread;
      // On the bot's own page, its chat is already there: it shows the message's thread.
      if (!own) panel.show(bot.id, thread);
      else if (thread && thread !== shown) void navigate({ to: "/bots/$id", params: { id: bot.id }, search: { thread } as never, replace: true });
    } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  // Pasted screenshots join the launch's attachments.
  const withShots = () => {
    if (files.length) useComposer.getState().update((s) => ({ ...s, attachments: [...s.attachments, ...files] }));
    shots.clear();
  };
  const launch = () => {
    if (reading) return;
    const c = useComposer.getState();
    const t = text.trim();
    // A kept draft, or nothing to launch: open the composer instead.
    if (!t || c.open || c.spec.text.trim()) { openOptions(ctx.launch, t); withShots(); setText(""); return; }
    c.launchNow(ctx.launch, t);
    withShots();
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
  return (
    <div className="flex min-w-[280px] max-w-text flex-1 items-center gap-1.5" data-tour="topbar.field">
      <div className="flex min-w-0 flex-1 items-center gap-1.5 overflow-hidden rounded-control border border-line bg-paper px-2 py-1 focus-within:border-teal">
        <Icon name="spark" size={14} className="flex-none text-teal-ink" />
        {ctx.label && <span className="flex min-w-[28px] max-w-[180px] shrink items-center gap-1 rounded bg-sunk px-1.5 text-[11px] text-ink-2" title="The page you are on goes with Ask and Launch">
          <span className="truncate">{ctx.label}</span>
          <button className="flex-none text-ink-3 hover:text-ink" aria-label="Leave out the page" title="Leave out the page" onClick={() => setWithContext(false)}><Icon name="x" size={12} /></button>
        </span>}
        {files.map((file, i) => (
          <span key={i} className="relative flex-none">
            <ImageThumb src={attachmentSrc(file.data_url)} name={file.name} className="h-6 max-w-12 rounded border border-line object-cover" />
            <button className="absolute -top-1 -right-1 grid size-3.5 place-items-center rounded-full bg-ink text-paper" aria-label={`Remove ${file.name}`} title={`Remove ${file.name}`} disabled={busy} onClick={() => shots.remove(i)}><Icon name="x" size={12} /></button>
          </span>
        ))}
        <textarea ref={ref} rows={1} value={text} disabled={busy} aria-label={bot ? "Message to a bot, or prompt for a new agent" : "Prompt for a new agent"}
          placeholder={bot ? "Ask, or launch an agent…" : "Launch an agent…"}
          title={`Paste a screenshot to attach it. ${ATTACH_LIMITS}`}
          onChange={(e) => setText(e.target.value)}
          onPaste={(e) => { if (busy) return; const pasted = pastedImages(e); if (pasted) void shots.add(pasted); }}
          onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) { e.preventDefault(); primary(); } }}
          className="min-h-6 min-w-[48px] flex-1 resize-none bg-transparent placeholder-shown:overflow-hidden placeholder-shown:whitespace-nowrap py-0.5 text-[13px] leading-5 outline-none" />
      </div>
      {bot && <span className="inline-flex min-w-0 flex-none" data-tour="topbar.ask">
        <button className={`btn btn-primary max-w-[180px] ${bots.length > 1 ? "rounded-r-none" : ""}`} title={`Ask ${bot.settings.name}: the answer opens ${own ? "in this chat" : "in the Ask panel"}${ctx.line ? ", with the page as context" : ""}`} disabled={busy || reading} onClick={() => void ask()}>
          <Icon name="send" size={12} /><span className="min-w-0 truncate">{bot.settings.name}</span>
        </button>
        {bots.length > 1 && <Popover viewport align="right" width={260} trigger={({ open: menu, toggle }) => (
          <button className="btn btn-primary rounded-l-none border-l-white/30 px-1.5!" aria-label="Choose the bot Ask sends to" title="Choose the bot Ask sends to" aria-expanded={menu} disabled={busy} onClick={toggle}><Icon name="chev" size={12} /></button>
        )}>
          {(close) => bots.map((b) => (
            <MenuItem key={b.id} active={b.id === bot.id} icon={<BotAvatar name={b.settings.name} size={16} />} label={b.settings.name}
              detail={b.id === ctx.botId ? "This page's bot" : b.id === askTarget(allBots, workspaceId)?.id ? "The workspace's Ask bot" : undefined}
              onClick={() => { close(); pick(b.id); ref.current?.focus(); }} />
          ))}
        </Popover>}
      </span>}
      <LaunchSplitButton ctx={ctx.launch} text={text} onLaunch={launch} onUsed={() => { withShots(); setText(""); }} primary={!bot}
        title={text.trim() ? `Launch an agent at once${ctx.launch ? " with the page as context" : ""}, with the folder's defaults` : "Open the composer"} />
    </div>
  );
}
