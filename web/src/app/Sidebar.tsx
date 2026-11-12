import { useKnowledgeUnseen } from "@/lib/knowledge";
import { useInboxNeeds } from "@/features/inbox/data";
import { useChannelAsks, useTodayCount } from "@/features/today/data";
import { botInitial, botState, botThread, useBots } from "@/lib/bots";
import type { Bot } from "@/gen/Bot";
import { useScope } from "@/lib/workspace";
import { archivedOf, channelsOf, useChannels } from "@/lib/channels";
import { ChannelLink } from "@/features/channels/ChannelRows";
import { WorkspaceMenu } from "@/features/workspaces/WorkspaceMenu";
import { Link } from "@tanstack/react-router";
import { memo, type ReactNode } from "react";
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
import { useSidebar, type SidebarGroup } from "@/stores/sidebar";

/** A corner mark of an icon in the narrow sidebar: an amber count, or a state dot. */
type Mark = { count: number } | "needs" | "paused" | "running" | "new" | null;

function Corner({ mark }: { mark: Mark }) {
  if (!mark) return null;
  if (typeof mark === "object") return mark.count > 0 ? <span className="absolute -top-2 -right-2.5 min-w-[17px] rounded-full border-2 border-chrome bg-attn px-[3px] text-center text-[11px] leading-[13px] font-bold text-white tabular-nums" aria-hidden>{mark.count}</span> : null;
  const tone = mark === "needs" ? "bg-attn" : mark === "paused" ? "border border-attn bg-chrome" : mark === "running" ? "bg-work" : "bg-ink-3";
  return <span className={`absolute -top-0.5 -right-1 size-[7px] rounded-full ${tone}`} aria-hidden />;
}

/** A place of the sidebar. Narrow: its icon alone, the label in the tooltip and a corner mark instead of `end`. */
function NavItem({ to, icon, children, end, tour, narrow, mark = null, title }: { to: string; icon: IconName; children: ReactNode; end?: ReactNode; tour?: string; narrow?: boolean; mark?: Mark; title?: string }) {
  if (narrow) return (
    <Link
      to={to}
      data-tour={tour}
      title={title}
      aria-label={title}
      className="mx-auto grid size-[31px] flex-none place-items-center rounded-control text-ink-2 no-underline hover:bg-sunk"
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
      className="flex min-h-[31px] flex-none items-center gap-2 rounded-control px-[9px] py-1 text-ink-2 no-underline hover:bg-sunk"
      activeProps={{ className: "!bg-paper !text-ink font-semibold shadow-[inset_0_0_0_1px_var(--color-line)]" }}
      activeOptions={{ exact: to === "/", includeSearch: false }}
    >
      <Icon name={icon} />
      {children}
      {end && <span className="ml-auto">{end}</span>}
    </Link>
  );
}

/**
 * A collapsible group of the sidebar (Bots): the chevron in the icon column shows or hides its rows
 * (remembered on this device), the label opens its page. `end` shows while it is collapsed (counts, state dots).
 */
function Group({ id, to, label, end, actions, tour, children }: { id: SidebarGroup; to: string; label: string; end?: ReactNode; actions?: ReactNode; tour?: string; children: ReactNode }) {
  const open = !useSidebar((s) => s.collapsed[id]);
  const toggle = useSidebar((s) => s.toggle);
  return (
    <div className="flex flex-none flex-col gap-px" data-tour={tour}>
      <div className="flex min-h-[31px] flex-none items-center rounded-control hover:bg-sunk">
        <button type="button" className="grid h-[31px] w-[34px] flex-none place-items-center rounded-control text-ink-3 hover:text-ink" aria-expanded={open} aria-label={`${open ? "Collapse" : "Expand"} ${label}`} title={`${open ? "Collapse" : "Expand"} ${label} on this device`} onClick={() => toggle(id)}>
          <Icon name="chev" size={12} className={`transition-transform ${open ? "" : "-rotate-90"}`} />
        </button>
        <Link to={to} className="flex h-[31px] min-w-0 flex-1 items-center gap-1.5 pr-1 text-ink-2 no-underline" activeProps={{ className: "!text-ink font-semibold" }} activeOptions={{ exact: true }}>
          <span className="flex-none">{label}</span>
          {!open && end && <span className="ml-auto flex-none">{end}</span>}
        </Link>
        {actions && <span className="flex flex-none items-center pr-0.5">{actions}</span>}
      </div>
      {open && children}
    </div>
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

/** A bot's row: its initial, its name and the state of its current session (a dot, never a count). */
const BotLink = memo(function BotLink({ bot, thread, narrow }: { bot: Bot; thread: Session[]; narrow?: boolean }) {
  const state = botState(thread);
  if (narrow) return (
    <Link to="/bots/$id" params={{ id: bot.id }} title={bot.settings.name} aria-label={bot.settings.name} className="mx-auto grid size-[31px] flex-none place-items-center rounded-control no-underline hover:bg-sunk" activeProps={{ className: "!bg-paper shadow-[inset_0_0_0_1px_var(--color-line)]" }}>
      <span className="relative grid size-[18px] place-items-center rounded-sm bg-teal-soft text-[11px] font-semibold text-teal-ink">{botInitial(bot.settings.name)}<Corner mark={state === "needs" || state === "paused" || state === "running" ? state : null} /></span>
    </Link>
  );
  return (
    <Link to="/bots/$id" params={{ id: bot.id }} className="flex h-[28px] items-center gap-2 rounded-control px-[9px] text-[13px] text-ink no-underline hover:bg-sunk" activeProps={{ className: "!bg-paper shadow-[inset_0_0_0_1px_var(--color-line)] font-semibold" }}>
      <span className="grid size-[16px] flex-none place-items-center rounded-sm bg-teal-soft text-[11px] font-semibold text-teal-ink" aria-hidden>{botInitial(bot.settings.name)}</span>
      <span className="min-w-0 flex-1 truncate">{bot.settings.name}</span>
      {state === "needs" && <span className="size-[7px] rounded-full bg-attn" title="Needs you" />}
      {state === "paused" && <span className="size-[7px] rounded-full border border-attn" title="Quota paused" />}
      {state === "running" && <span className="lp-spin" title="Working" />}
    </Link>
  );
});

/** A small label over a part of the Bots group (Channels, Bot DMs). */
const SubLabel = ({ children }: { children: ReactNode }) => <div className="mt-1.5 px-[9px] pb-0.5 text-[11px] font-semibold text-ink-3">{children}</div>;

/** Bots of the workspace (Ask Launchpad among them), then its channels and bot DMs. Collapsed: one state dot (and the
 * amber dot of a channel that asks you); their proposals wait in Today, and a channel only when a bot asks you there. */
function BotList({ sessions, narrow }: { sessions: Session[]; narrow: boolean }) {
  const scope = useScope();
  const bots = (useBots().data ?? []).filter((b) => b.workspace_id === scope.workspace.id);
  const allChannels = useChannels().data;
  const { channels, dms } = channelsOf(allChannels, scope.workspace.id);
  const archived = archivedOf(allChannels, scope.workspace.id).length;
  const asks = useChannelAsks();
  if (narrow) return (
    <div className="flex flex-none flex-col gap-px" data-tour="sidebar.bots">
      {bots.map((b) => <BotLink key={b.id} bot={b} thread={botThread(sessions, b.id)} narrow />)}
      {bots.length === 0 && <Link to="/bots" search={{ new: true } as never} className="mx-auto grid size-[31px] place-items-center rounded-control text-ink-3 no-underline hover:bg-sunk hover:text-ink" title="New bot" aria-label="New bot"><Icon name="plus" size={14} /></Link>}
    </div>
  );
  const states = bots.map((b) => botState(botThread(sessions, b.id)));
  const summary = <span className="flex items-center gap-1.5">
    {states.includes("needs") && <span className="size-[7px] rounded-full bg-attn" title="A bot needs you" />}
    {states.includes("paused") && <span className="size-[7px] rounded-full border border-attn" title="A bot is quota paused" />}
    {states.includes("running") && <span className="lp-spin" title="A bot works" />}
    {asks.size > 0 && <span className="size-[7px] rounded-full bg-attn" title="A channel message asks you" />}
  </span>;
  return (
    <Group id="bots" to="/bots" label="Bots" tour="sidebar.bots" end={summary} actions={<Link to="/bots" search={{ new: true } as never} className="iconbtn" title="New bot" aria-label="New bot"><Icon name="plus" size={14} /></Link>}>
      {bots.map((b) => <BotLink key={b.id} bot={b} thread={botThread(sessions, b.id)} />)}
      {bots.length === 0 && <Link to="/bots" search={{ new: true } as never} className="mx-[9px] my-1 text-[12px] text-teal-ink no-underline hover:underline">+ New bot</Link>}
      {channels.length > 0 && <div className="flex flex-col gap-px" data-tour="sidebar.channels"><SubLabel>Channels</SubLabel>{channels.map((c) => <ChannelLink key={c.id} c={c} />)}</div>}
      {dms.length > 0 && <div className="flex flex-col gap-px" data-tour={channels.length ? undefined : "sidebar.channels"}><SubLabel>Bot DMs</SubLabel>{dms.map((c) => <ChannelLink key={c.id} c={c} />)}</div>}
      {archived > 0 && <Link to="/bots" search={{ archived: true } as never} className="mt-1 flex h-[28px] items-center gap-2 rounded-control px-[9px] text-[12px] text-ink-3 no-underline hover:bg-sunk hover:text-ink" data-tour="sidebar.archived"><span className="min-w-0 flex-1 truncate">Archived channels</span><Count n={archived} /></Link>}
    </Group>
  );
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
  const needs = useInboxNeeds();
  const today = useTodayCount();
  const knowledgeNew = useKnowledgeUnseen();
  const scope = useScope();
  const projects = scope.projects;
  const sessions = (useSessions().data ?? []).filter(scope.hasSession);
  const running = sessions.filter(isRunning).length;
  const quotaPaused = sessions.filter(isQuotaPaused).length;
  const folderSessions = sessions.filter((s) => projects.some((p) => inProject(p, s)));
  const todayTitle = `${today} decisions wait: questions, finished agents, proposals, drafts to send and reviews`;
  const inboxTitle = `${needs} for you: unread review requests, mentions and assignments. The Inbox "For you" filter lists them.`;
  const knowledgeTitle = `${knowledgeNew} changes by agents since your last look`;
  const label = (name: string, detail: string | false) => detail ? `${name} · ${detail}` : name;
  const collapse = (
    <button type="button" className={`grid size-[31px] flex-none place-items-center rounded-control text-ink-2 hover:bg-sunk hover:text-ink ${narrow ? "mx-auto" : ""}`} aria-label={narrow ? "Expand sidebar" : "Collapse sidebar"} aria-expanded={!narrow} title={narrow ? "Expand the sidebar on this device" : "Collapse the sidebar to icons on this device"} onClick={() => toggleGroup("sidebar")}>
      <Icon name="sidebar" size={16} />
    </button>
  );
  return (
    <aside className={`flex flex-none flex-col border-r border-line bg-chrome ${narrow ? "w-[46px]" : "w-[200px]"}`}>
      <WorkspaceMenu narrow={narrow} />
      {/* One scroll area for the groups; Settings stays at the bottom. */}
      <div className={`relative flex min-h-0 flex-1 flex-col gap-px overflow-y-auto pt-2.5 pb-2 ${narrow ? "overflow-x-hidden px-1 [scrollbar-width:none]" : "px-2"}`}>
      <BotList sessions={sessions} narrow={narrow} />
      <nav aria-label="Places" data-tour="sidebar.nav" className="mt-2 flex flex-none flex-col gap-px border-t border-line-2 pt-2">
      <NavItem narrow={narrow} tour="sidebar.today" to="/" icon="okcircle" mark={{ count: today }} title={label("Today", today > 0 && todayTitle)} end={<Count n={today} needs title={todayTitle} />}>
        Today
      </NavItem>
      <NavItem narrow={narrow} tour="sidebar.agents" to="/agents" icon="agents" mark={quotaPaused ? "paused" : running ? "running" : null} title={label("Agents", [quotaPaused && `${quotaPaused} quota paused`, running && `${running} running`].filter(Boolean).join(", "))} end={<AgentCounts quotaPaused={quotaPaused} running={running} />}>
        Agents
      </NavItem>
      <NavItem narrow={narrow} tour="sidebar.inbox" to="/inbox" icon="inbox" mark={{ count: needs }} title={label("Inbox", needs > 0 && inboxTitle)} end={<Count n={needs} needs title={inboxTitle} />}>
        Inbox
      </NavItem>
      <NavItem narrow={narrow} tour="sidebar.schedules" to="/schedules" icon="clock" title="Schedules">
        Schedules
      </NavItem>
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
  );
}
