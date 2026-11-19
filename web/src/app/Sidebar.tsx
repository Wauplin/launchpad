import { useKnowledgeUnseen } from "@/lib/knowledge";
import { useFolios } from "@/lib/folios";
import { useTodayCount } from "@/features/today/data";
import { useScope } from "@/lib/workspace";
import { usePinnedChats, useChats, useChatsNeedCount } from "@/features/chats/data";
import { ChatMark } from "@/features/chats/ChatMark";
import { chatNeedsYou, type Chat } from "@/lib/chatList";
import { memo } from "react";
import { WorkspaceMenu } from "@/features/workspaces/WorkspaceMenu";
import { Link, useRouterState } from "@tanstack/react-router";
import { type ReactNode } from "react";
import type { Project } from "@/gen/Project";
import { isRunning, isQuotaPaused, needsYou } from "@/lib/sessionStatus";
import type { Session } from "@/gen/Session";
import { Icon, type IconName } from "@/components/Icon";
import { Count } from "@/components/ui";
import { toastError } from "@/components/Toast";
import { useSessions } from "@/lib/queries";
import { useTheme } from "@/stores/theme";
import { useSetupSummary } from "@/features/setup/summary";
import { useOnboarding, usePatchOnboarding } from "@/onboarding/data";
import { useColumn } from "@/components/ResizeHandle";
import { useSidebar } from "@/stores/sidebar";

/** A corner mark of an icon in the narrow sidebar: an amber count, or a state dot. */
type Mark = { count: number } | "needs" | "paused" | "running" | "new" | null;

function Corner({ mark }: { mark: Mark }) {
  if (!mark) return null;
  if (typeof mark === "object") return mark.count > 0 ? <span className="absolute -top-2 -right-2.5 min-w-[17px] rounded-full border-2 border-chrome bg-attn px-[3px] text-center text-[11px] leading-[13px] font-bold text-white tabular-nums" aria-hidden>{mark.count}</span> : null;
  const tone = mark === "needs" ? "bg-attn" : mark === "paused" ? "border border-attn bg-chrome" : mark === "running" ? "bg-work" : "bg-ink-3";
  return <span className={`absolute -top-0.5 -right-1 size-[7px] rounded-full ${tone}`} aria-hidden />;
}

/** A place of the sidebar. Narrow: its icon alone, the label in the tooltip and a corner mark instead of `end`. */
function NavItem({ to, icon, children, end, tour, narrow, mark = null, title, current }: { to: string; icon: IconName; children: ReactNode; end?: ReactNode; tour?: string; narrow?: boolean; mark?: Mark; title?: string; current?: boolean }) {
  // `current`: the place shows although the address is one of its pages (Chats: a chat, a bot's settings).
  const on = current ? (narrow ? " !bg-paper !text-ink shadow-[inset_0_0_0_1px_var(--color-line)]" : " !bg-paper !text-ink font-semibold shadow-[inset_0_0_0_1px_var(--color-line)]") : "";
  if (narrow) return (
    <Link
      to={to}
      data-tour={tour}
      title={title}
      aria-label={title}
      className={`mx-auto grid size-[31px] flex-none place-items-center rounded-control text-ink-2 no-underline hover:bg-sunk${on}`}
      activeProps={{ className: "!bg-paper !text-ink shadow-[inset_0_0_0_1px_var(--color-line)]" }}
      activeOptions={{ exact: to === "/", includeSearch: false }}
    >
      <span className="relative grid place-items-center"><Icon name={icon} /><Corner mark={mark} /></span>
    </Link>
  );
  return (
    <Link
      to={to}
      data-tour={tour}
      className={`flex min-h-[31px] flex-none items-center gap-2 rounded-control px-[9px] py-1 text-ink-2 no-underline hover:bg-sunk${on}`}
      activeProps={{ className: "!bg-paper !text-ink font-semibold shadow-[inset_0_0_0_1px_var(--color-line)]" }}
      activeOptions={{ exact: to === "/", includeSearch: false }}
    >
      <Icon name={icon} />
      {children}
      {end && <span className="ml-auto">{end}</span>}
    </Link>
  );
}

const inProject = (p: Project, s: Session) => s.project_id === p.id || s.cwd === p.path || s.cwd.startsWith(p.path + "/");

/** State dots of a set of sessions (Folders): needs you, quota paused, running. */
function RunDots({ sessions }: { sessions: Session[] }) {
  return <span className="flex items-center gap-1.5">
    {sessions.some(needsYou) && <span className="size-[7px] rounded-full bg-attn" title="Needs you" />}
    {sessions.some(isQuotaPaused) && <span className="size-[7px] rounded-full border border-attn" title="Quota paused" />}
    {sessions.some(isRunning) && <span className="lp-spin" title="Running" />}
  </span>;
}

/** The one state dot of a set of sessions in the narrow sidebar, in the order of RunDots. */
const runMark = (sessions: Session[]): Mark => sessions.some(needsYou) ? "needs" : sessions.some(isQuotaPaused) ? "paused" : sessions.some(isRunning) ? "running" : null;

/** "Finish setup · N left" until the required setup items pass or the owner dismisses it. */
function FinishSetup({ narrow }: { narrow: boolean }) {
  const left = useSetupSummary().data?.required_left ?? 0;
  const onboarding = useOnboarding().data;
  const patch = usePatchOnboarding();
  if (!onboarding || left === 0 || onboarding.dismissed.includes("setup")) return null;
  if (narrow) return <NavItem narrow to="/setup" icon="check" mark={{ count: left }} title={`Finish setup · ${left} left`}>Finish setup</NavItem>;
  return (
    <div className="flex items-center gap-0.5">
      <div className="min-w-0 flex-1">
        <NavItem to="/setup" icon="check" end={<span className="rounded-full bg-attn-soft px-1.5 text-[11px] font-semibold whitespace-nowrap text-attn tabular-nums">{left} left</span>}><span className="whitespace-nowrap">Finish setup</span></NavItem>
      </div>
      <button type="button" className="iconbtn" title="Hide Finish setup (the checklist stays in Settings → Onboarding)" aria-label="Hide Finish setup" onClick={() => void patch({ dismiss: ["setup"] }).catch(toastError)}><Icon name="x" size={12} /></button>
    </div>
  );
}

/** A pinned chat under Chats: its mark, its title and a state mark. Narrow: its mark alone. */
const PinnedChat = memo(function PinnedChat({ chat, narrow }: { chat: Chat; narrow: boolean }) {
  const link = chat.kind === "thread" ? { to: "/bots/$id" as const, params: { id: chat.bot.id }, search: { thread: chat.thread } } : { to: "/channels/$id" as const, params: { id: chat.id }, search: {} };
  const needs = chatNeedsYou(chat);
  const running = chat.kind === "thread" && chat.section === "Running";
  const bold = needs || (chat.kind === "channel" && chat.unread > 0);
  const state = needs ? "needs you" : running ? "working" : chat.kind === "channel" && chat.unread ? `${chat.unread} unread` : null;
  const title = `${chat.title}${state ? ` · ${state}` : ""} (pinned)`;
  const active = { className: "!bg-paper shadow-[inset_0_0_0_1px_var(--color-line)] !text-ink" };
  if (narrow) return <Link {...link} title={title} aria-label={title} className="mx-auto grid size-[31px] flex-none place-items-center rounded-control no-underline hover:bg-sunk" activeProps={active} activeOptions={{ exact: true, includeSearch: true }}>
    <span className="relative grid place-items-center"><ChatMark chat={chat} size={16} /><Corner mark={needs ? "needs" : running ? "running" : null} /></span>
  </Link>;
  return <Link {...link} title={title} aria-label={title} className="flex h-[28px] flex-none items-center gap-2 rounded-control pr-[9px] pl-[22px] text-[13px] text-ink-2 no-underline hover:bg-sunk" activeProps={active} activeOptions={{ exact: true, includeSearch: true }}>
    <ChatMark chat={chat} size={16} />
    <span className={`min-w-0 flex-1 truncate ${bold ? "font-semibold text-ink" : ""}`}>{chat.title}</span>
    {needs ? <span className="size-[7px] flex-none rounded-full bg-attn" aria-hidden /> : running ? <span className="lp-spin" aria-hidden /> : chat.kind === "channel" && chat.unread > 0 ? <span className="size-[7px] flex-none rounded-full bg-ink-3" aria-hidden /> : null}
  </Link>;
});

/** Chats, under Agents: the count of chats that need you (their Today cards: a question, an unseen answer, a channel
 * that asks you), then only the chats you pinned, each a shortcut. */
function SidebarChats({ narrow }: { narrow: boolean }) {
  const { chats } = useChats();
  const path = useRouterState({ select: (st) => st.location.pathname });
  const inChats = /^\/(bots|channels)(\/|$)/.test(path);
  const pinned = usePinnedChats();
  const needs = useChatsNeedCount();
  const running = chats.some((c) => c.kind === "thread" && c.section === "Running");
  const detail = [needs && `${needs} need${needs === 1 ? "s" : ""} you`, running && "a bot works"].filter(Boolean).join(", ");
  return <>
    <NavItem narrow={narrow} tour="sidebar.chats" to="/chats" icon="comment" current={inChats} mark={needs ? { count: needs } : running ? "running" : null} title={detail ? `Chats · ${detail}` : "Chats"}
      end={<span className="flex items-center gap-1">{running && <span className="lp-spin" title="A bot works" />}<Count n={needs} needs title={`${needs} chat${needs === 1 ? "" : "s"} need${needs === 1 ? "s" : ""} you`} /></span>}>
      Chats
    </NavItem>
    {pinned.length > 0 && <div className="flex flex-none flex-col gap-px" role="group" aria-label="Pinned chats" data-tour="sidebar.pinned">
      {pinned.map((c) => <PinnedChat key={c.pin} chat={c} narrow={narrow} />)}
    </div>}
  </>;
}

/** Agents in the sidebar: a monitor, with the quota-paused and running counts. What needs you is in Today. */
function AgentCounts({ quotaPaused, running }: { quotaPaused: number; running: number }) {
  if (!quotaPaused && !running) return null;
  return <span className="flex items-center gap-1">
    <Count n={quotaPaused} needs title={`${quotaPaused} quota paused`} />
    {running > 0 && <span className="flex items-center" title={`${running} running`}><span className="lp-spin" /><Count n={running} title={`${running} running`} /></span>}
  </span>;
}

export function Sidebar() {
  const { theme, toggle } = useTheme();
  const narrow = !!useSidebar((s) => s.collapsed.sidebar);
  const toggleGroup = useSidebar((s) => s.toggle);
  const today = useTodayCount();
  const knowledgeNew = useKnowledgeUnseen();
  const scope = useScope();
  const folioUnsent = (useFolios(scope.workspace.id).data ?? []).reduce((n, f) => n + f.unsent, 0);
  const projects = scope.projects;
  const sessions = (useSessions().data ?? []).filter(scope.hasSession);
  const running = sessions.filter(isRunning).length;
  const quotaPaused = sessions.filter(isQuotaPaused).length;
  const folderSessions = sessions.filter((s) => projects.some((p) => inProject(p, s)));
  const todayTitle = `${today} decisions wait: questions, finished agents, proposals, drafts to send and reviews`;
  const knowledgeTitle = `${knowledgeNew} changes by agents since your last look`;
  const label = (name: string, detail: string | false) => detail ? `${name} · ${detail}` : name;
  const column = useColumn("sidebar", "sidebar");
  const collapse = (
    <button type="button" className={`grid size-[31px] flex-none place-items-center rounded-control text-ink-2 hover:bg-sunk hover:text-ink ${narrow ? "mx-auto" : ""}`} aria-label={narrow ? "Expand sidebar" : "Collapse sidebar"} aria-expanded={!narrow} title={narrow ? "Expand the sidebar on this device" : "Collapse the sidebar to icons on this device"} onClick={() => toggleGroup("sidebar")}>
      <Icon name="sidebar" size={16} />
    </button>
  );
  return (
    <>
    <aside className={`flex flex-none flex-col bg-chrome ${narrow ? "w-[46px] border-r border-line" : ""}`} style={narrow ? undefined : { width: column.width }}>
      <WorkspaceMenu narrow={narrow} />
      {/* One scroll area for the groups; Settings stays at the bottom. */}
      <div className={`relative flex min-h-0 flex-1 flex-col gap-px overflow-y-auto pt-2.5 pb-2 ${narrow ? "overflow-x-hidden px-1 [scrollbar-width:none]" : "px-2"}`}>
      <nav aria-label="Places" data-tour="sidebar.nav" className="flex flex-none flex-col gap-px">
      <NavItem narrow={narrow} tour="sidebar.today" to="/" icon="okcircle" mark={{ count: today }} title={label("Today", today > 0 && todayTitle)} end={<Count n={today} needs title={todayTitle} />}>
        Today
      </NavItem>
      <NavItem narrow={narrow} tour="sidebar.agents" to="/agents" icon="agents" mark={quotaPaused ? "paused" : running ? "running" : null} title={label("Agents", [quotaPaused && `${quotaPaused} quota paused`, running && `${running} running`].filter(Boolean).join(", "))} end={<AgentCounts quotaPaused={quotaPaused} running={running} />}>
        Agents
      </NavItem>
      <SidebarChats narrow={narrow} />
      <NavItem narrow={narrow} tour="sidebar.schedules" to="/schedules" icon="clock" title="Schedules">
        Schedules
      </NavItem>
      <NavItem narrow={narrow} tour="sidebar.folios" to="/folios" icon="folio" mark={folioUnsent ? "new" : null} title={label("Folios", folioUnsent > 0 && `${folioUnsent} marks not sent yet`)} end={<Count n={folioUnsent} title={`${folioUnsent} marks not sent yet`} />}>Folios</NavItem>
      <NavItem narrow={narrow} to="/topics" icon="tag" title="Topics">Topics</NavItem>
      <NavItem narrow={narrow} tour="sidebar.knowledge" to="/knowledge" icon="book" mark={knowledgeNew ? "new" : null} title={label("Knowledge", knowledgeNew > 0 && knowledgeTitle)} end={<Count n={knowledgeNew} title={knowledgeTitle} />}>
        Knowledge
      </NavItem>
      <NavItem narrow={narrow} tour="sidebar.folders" to="/projects" icon="folder" mark={runMark(folderSessions)} title="Folders" end={<RunDots sessions={folderSessions} />}>
        Folders
      </NavItem>
      </nav>
      </div>
      <div className={`flex flex-none flex-col gap-px border-t border-line-2 pt-1.5 pb-2.5 ${narrow ? "px-1.5" : "px-2"}`}>
      <FinishSetup narrow={narrow} />
      {narrow ? <>
        <NavItem narrow to="/settings" icon="sliders" title="Settings">Settings</NavItem>
        {collapse}
      </> : <div className="flex items-center gap-1">
        <div className="min-w-0 flex-1">
          <NavItem to="/settings" icon="sliders">
            Settings
          </NavItem>
        </div>
        <button type="button" className="flex size-[31px] flex-none items-center justify-center rounded-control text-ink-2 hover:bg-sunk" aria-label="Dark theme" aria-pressed={theme === "dark"} title={`Switch to ${theme === "light" ? "dark" : "light"} theme on this device`} onClick={toggle}>
          <Icon name={theme === "dark" ? "moon" : "sun"} size={16} />
        </button>
        {collapse}
      </div>}
      </div>
    </aside>
    {!narrow && column.handle}
    </>
  );
}
