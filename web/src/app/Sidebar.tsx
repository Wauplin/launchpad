import { useKnowledgeUnseen } from "@/lib/knowledge";
import { useInboxNeeds } from "@/features/inbox/data";
import { useTodayCount } from "@/features/today/data";
import { botInitial, botState, botThread, useBots } from "@/lib/bots";
import type { Bot } from "@/gen/Bot";
import { useScope } from "@/lib/workspace";
import { WorkspaceMenu } from "@/features/workspaces/WorkspaceMenu";
import { Link, useNavigate } from "@tanstack/react-router";
import { memo, useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { Project } from "@/gen/Project";
import { isRunning, isQuotaPaused, needsYou } from "@/lib/sessionStatus";
import type { Session } from "@/gen/Session";
import { Icon, type IconName } from "@/components/Icon";
import { toastError } from "@/components/Toast";
import { formatAge } from "@/lib/format";
import { useSessions } from "@/lib/queries";
import { useTheme } from "@/stores/theme";
import { useComposer } from "@/stores/composer";
import { useOpenTerminal, useTerminalsHere } from "@/features/terminals/lib";
import { useAddFolder } from "@/features/projects/addFolderStore";
import { useSetupSummary } from "@/features/setup/summary";
import { useOnboarding, usePatchOnboarding } from "@/onboarding/data";
import { useSidebar, type SidebarGroup } from "@/stores/sidebar";

function NavItem({ to, icon, children, end, tour }: { to: string; icon: IconName; children: ReactNode; end?: ReactNode; tour?: string }) {
  return (
    <Link
      to={to}
      data-tour={tour}
      className="flex min-h-[31px] flex-none items-center gap-[9px] rounded-[7px] px-[9px] py-1 text-ink-2 no-underline hover:bg-sunk"
      activeProps={{ className: "!bg-paper !text-ink font-semibold shadow-[inset_0_0_0_1px_var(--color-line)]" }}
      activeOptions={{ exact: to === "/" }}
    >
      <Icon name={icon} />
      {children}
      {end && <span className="ml-auto">{end}</span>}
    </Link>
  );
}

export function SideLabel({ children, end }: { children: ReactNode; end?: ReactNode }) {
  return <div className="flex min-h-[28px] items-center gap-1 px-[9px] pt-3 pb-0.5"><span className="seclabel min-w-0 flex-1 truncate">{children}</span>{end}</div>;
}

/**
 * A collapsible group of the sidebar (Bots, Folders): the chevron in the icon column shows or hides its rows
 * (remembered on this device), the label opens its page. `end` shows while it is collapsed (counts, state dots).
 */
function Group({ id, to, label, end, actions, tour, children }: { id: SidebarGroup; to: string; label: string; end?: ReactNode; actions?: ReactNode; tour?: string; children: ReactNode }) {
  const open = !useSidebar((s) => s.collapsed[id]);
  const toggle = useSidebar((s) => s.toggle);
  return (
    <div className="flex flex-none flex-col gap-px" data-tour={tour}>
      <div className="flex min-h-[31px] flex-none items-center rounded-[7px] hover:bg-sunk">
        <button type="button" className="grid h-[31px] w-[34px] flex-none place-items-center rounded-[7px] text-ink-3 hover:text-ink" aria-expanded={open} aria-label={`${open ? "Collapse" : "Expand"} ${label}`} title={`${open ? "Collapse" : "Expand"} ${label} on this device`} onClick={() => toggle(id)}>
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
/** Needs you, quota paused or running: the states the sidebar marks. */
const active = (s: Session) => needsYou(s) || isQuotaPaused(s) || isRunning(s);

/** State dots of a set of sessions: needs you, quota paused, running. */
function RunDots({ sessions }: { sessions: Session[] }) {
  return <span className="flex items-center gap-1.5">
    {sessions.some(needsYou) && <span className="size-[7px] rounded-full bg-attn" title="Needs you" />}
    {sessions.some(isQuotaPaused) && <span className="size-[7px] rounded-full border border-attn" title="Quota paused" />}
    {sessions.some(isRunning) && <span className="lp-spin" title="Running" />}
  </span>;
}

function RunDot({ p, sessions }: { p: Project; sessions: Session[] }) {
  return <RunDots sessions={sessions.filter((s) => inProject(p, s))} />;
}

const ROW = 28;

/** A folder row: the link, its state dots, and quick actions on hover or focus (always visible on touch): Files · Terminal · Launch. */
const FolderLink = memo(function FolderLink({ p, sessions }: { p: Project; sessions: Session[] }) {
  const navigate = useNavigate();
  const terminal = useOpenTerminal();
  const terminalsHere = useTerminalsHere();
  const start = useComposer((s) => s.start);
  const name = p.is_temp ? p.name.replace(/^tmp-\d{4}-/, "") || p.name : p.name;
  return (
    <div className="group/folder relative flex h-[28px] flex-none items-center rounded-md hover:bg-sunk focus-within:bg-sunk">
      <Link
        to="/projects/$id"
        params={{ id: p.id }}
        className={`flex h-full min-w-0 flex-1 items-center gap-2 rounded-md px-[9px] no-underline ${p.is_temp ? "text-[12px] text-ink-2" : "text-[13px] text-ink"}`}
        activeProps={{ className: "!bg-paper shadow-[inset_0_0_0_1px_var(--color-line)]" }}
        activeOptions={{ exact: true }}
        title={p.is_temp ? `${p.name} · created ${formatAge(p.created_at)} ago` : p.path}
      >
        <span className="min-w-0 truncate">{name}</span>
        <span className="ml-auto group-hover/folder:hidden group-focus-within/folder:hidden [@media(hover:none)]:hidden"><RunDot p={p} sessions={sessions} /></span>
      </Link>
      <span className="absolute right-0.5 hidden items-center group-hover/folder:flex group-focus-within/folder:flex [@media(hover:none)]:static [@media(hover:none)]:flex">
        <button className="iconbtn size-[22px]!" title={`Files of ${name}`} aria-label={`Files of ${name}`} onClick={() => void navigate({ to: "/projects/$id/code", params: { id: p.id } })}><Icon name="folder" size={13} /></button>
        {terminalsHere && <button className="iconbtn size-[22px]!" title={`Terminal in ${name}`} aria-label={`Terminal in ${name}`} onClick={() => void terminal.shell(p.path)}><Icon name="term" size={13} /></button>}
        <button className="iconbtn size-[22px]!" title={`Launch an agent in ${name}`} aria-label={`Launch an agent in ${name}`} onClick={() => start({ projectId: p.id })}><Icon name="play" size={13} /></button>
      </span>
    </div>
  );
});

/** "Finish setup · N left" until the required setup items pass or the owner dismisses it. */
function FinishSetup() {
  const left = useSetupSummary().data?.required_left ?? 0;
  const onboarding = useOnboarding().data;
  const patch = usePatchOnboarding();
  if (!onboarding || left === 0 || onboarding.dismissed.includes("setup")) return null;
  return (
    <div className="flex items-center gap-0.5">
      <div className="min-w-0 flex-1">
        <NavItem to="/setup" icon="check" end={<span className="rounded-full bg-attn-soft px-1.5 text-[11px] font-semibold whitespace-nowrap text-attn tabular-nums">{left} left</span>}><span className="whitespace-nowrap">Finish setup</span></NavItem>
      </div>
      <button type="button" className="iconbtn size-[22px]!" title="Hide Finish setup (the checklist stays in Settings → Onboarding)" aria-label="Hide Finish setup" onClick={() => void patch({ dismiss: ["setup"] }).catch(toastError)}><Icon name="x" size={12} /></button>
    </div>
  );
}

/** Folders of the workspace, virtualized against the sidebar's scroll area with fixed row heights. */
function FolderList({ folders, sessions, scroller }: { folders: Project[]; sessions: Session[]; scroller: RefObject<HTMLDivElement | null> }) {
  const ref = useRef<HTMLDivElement>(null);
  const [margin, setMargin] = useState(0);
  // The list's offset in the scroll area moves when a group above it opens or closes.
  useLayoutEffect(() => { const top = ref.current?.offsetTop ?? 0; if (top !== margin) setMargin(top); });
  const v = useVirtualizer({ count: folders.length, getScrollElement: () => scroller.current, estimateSize: () => ROW, overscan: 6, scrollMargin: margin, useFlushSync: false });
  return (
    <div ref={ref} className="relative flex-none" style={{ height: v.getTotalSize() }}>
      {v.getVirtualItems().map((it) => <div key={folders[it.index].id} className="absolute inset-x-0" style={{ top: it.start - margin, height: ROW }}><FolderLink p={folders[it.index]} sessions={sessions} /></div>)}
    </div>
  );
}

/** A bot's row: its initial, its name and the state of its current session (a dot, never a count). */
const BotLink = memo(function BotLink({ bot, thread }: { bot: Bot; thread: Session[] }) {
  const state = botState(thread);
  return (
    <Link to="/bots/$id" params={{ id: bot.id }} className="flex h-[28px] items-center gap-2 rounded-md px-[9px] text-[13px] text-ink no-underline hover:bg-sunk" activeProps={{ className: "!bg-paper shadow-[inset_0_0_0_1px_var(--color-line)] font-semibold" }}>
      <span className="grid size-[16px] flex-none place-items-center rounded-[4px] bg-teal-soft text-[10px] font-semibold text-teal-ink" aria-hidden>{botInitial(bot.settings.name)}</span>
      <span className="min-w-0 flex-1 truncate">{bot.settings.name}</span>
      {state === "needs" && <span className="size-[7px] rounded-full bg-attn" title="Needs you" />}
      {state === "paused" && <span className="size-[7px] rounded-full border border-attn" title="Quota paused" />}
      {state === "running" && <span className="lp-spin" title="Working" />}
    </Link>
  );
});

/** Bots of the workspace (Ask Launchpad among them). Collapsed: one state dot; their proposals wait in Today. */
function BotList({ sessions }: { sessions: Session[] }) {
  const scope = useScope();
  const bots = (useBots().data ?? []).filter((b) => b.workspace_id === scope.workspace.id);
  const states = bots.map((b) => botState(botThread(sessions, b.id)));
  const summary = <span className="flex items-center gap-1.5">
    {states.includes("needs") && <span className="size-[7px] rounded-full bg-attn" title="A bot needs you" />}
    {states.includes("paused") && <span className="size-[7px] rounded-full border border-attn" title="A bot is quota paused" />}
    {states.includes("running") && <span className="lp-spin" title="A bot works" />}
  </span>;
  return (
    <Group id="bots" to="/bots" label="Bots" tour="sidebar.bots" end={summary} actions={<Link to="/bots" search={{ new: true } as never} className="iconbtn size-[22px]!" title="New bot" aria-label="New bot"><Icon name="plus" size={13} /></Link>}>
      {bots.map((b) => <BotLink key={b.id} bot={b} thread={botThread(sessions, b.id)} />)}
      {bots.length === 0 && <Link to="/bots" search={{ new: true } as never} className="mx-[9px] my-1 text-[12px] text-teal-ink no-underline hover:underline">+ New bot</Link>}
    </Group>
  );
}

/** Agents in the sidebar: a monitor, with the quota-paused and running counts. What needs you is in Today. */
function AgentCounts({ quotaPaused, running }: { quotaPaused: number; running: number }) {
  if (!quotaPaused && !running) return null;
  return <span className="flex items-center gap-1 text-[11px] font-semibold tabular-nums">
    {quotaPaused > 0 && <span className="rounded-full border border-attn-line px-1.5 leading-[14px] text-attn" title={`${quotaPaused} quota paused`} aria-label={`${quotaPaused} quota paused`}>{quotaPaused}</span>}
    {running > 0 && <span className="flex items-center gap-1 text-work" title={`${running} running`} aria-label={`${running} running`}><span className="lp-spin" />{running}</span>}
  </span>;
}

export function Sidebar() {
  const { theme, toggle } = useTheme();
  const needs = useInboxNeeds();
  const today = useTodayCount();
  const knowledgeNew = useKnowledgeUnseen();
  const scope = useScope();
  const showAdd = useAddFolder((s) => s.show);
  const projects = scope.projects;
  const sessions = (useSessions().data ?? []).filter(scope.hasSession);
  const [allTemp, setAllTemp] = useState(false);
  const running = sessions.filter(isRunning).length;
  const quotaPaused = sessions.filter(isQuotaPaused).length;
  const folders = projects.filter((p) => !p.is_temp).sort((a, b) => a.name.localeCompare(b.name));
  const temp = projects.filter((p) => p.is_temp).sort((a, b) => b.created_at - a.created_at);
  // Temporary folders: only those with agents that need you, run or wait for quota, unless expanded.
  const tempActive = temp.filter((p) => sessions.some((s) => inProject(p, s) && active(s)));
  const shownTemp = allTemp ? temp : tempActive;
  const scroller = useRef<HTMLDivElement>(null);
  return (
    <aside className="flex w-[200px] flex-none flex-col border-r border-line bg-chrome">
      {/* One scroll area for the groups; Settings stays at the bottom. */}
      <WorkspaceMenu />
      <div ref={scroller} className="relative flex min-h-0 flex-1 flex-col gap-px overflow-y-auto px-2 pt-2.5 pb-2">
      <BotList sessions={sessions} />
      <nav aria-label="Places" data-tour="sidebar.nav" className="mt-2 flex flex-none flex-col gap-px border-t border-line-2 pt-2">
      <NavItem tour="sidebar.today" to="/" icon="okcircle" end={today > 0 ? <span className="rounded-full bg-attn-soft px-1.5 text-[11px] font-semibold text-attn tabular-nums" title={`${today} decisions wait: questions, finished agents, proposals, drafts to send and reviews`} aria-label={`${today} waiting`}>{today}</span> : undefined}>
        Today
      </NavItem>
      <NavItem tour="sidebar.agents" to="/agents" icon="agents" end={<AgentCounts quotaPaused={quotaPaused} running={running} />}>
        Agents
      </NavItem>
      <NavItem tour="sidebar.inbox" to="/inbox" icon="inbox" end={needs > 0 ? <span className="text-[11px] font-semibold text-ink-3 tabular-nums" title={`${needs} for you: unread review requests, mentions and assignments. The Inbox "For you" filter lists them.`} aria-label={`${needs} for you`}>{needs}</span> : undefined}>
        Inbox
      </NavItem>
      <NavItem tour="sidebar.schedules" to="/schedules" icon="clock">
        Schedules
      </NavItem>
      <NavItem tour="sidebar.knowledge" to="/knowledge" icon="book" end={knowledgeNew > 0 ? <span className="text-[11px] font-semibold text-ink-3 tabular-nums" title={`${knowledgeNew} changes by agents since your last look`}>{knowledgeNew}</span> : undefined}>
        Knowledge
      </NavItem>
      <Group id="folders" to="/projects" label="Folders" tour="sidebar.folders" end={<RunDots sessions={sessions.filter((s) => projects.some((p) => inProject(p, s)))} />} actions={
        <button className="iconbtn size-[22px]!" title={`Add folders to ${scope.workspace.name}`} aria-label="Add folders" data-tour="sidebar.add-folder" onClick={() => showAdd()}><Icon name="plus" size={13} /></button>
      }>
        {folders.length === 0 && <button className="mx-[9px] my-1 text-left text-[12px] text-teal-ink hover:underline" onClick={() => showAdd()}>+ Add folder</button>}
        <FolderList folders={folders} sessions={sessions} scroller={scroller} />
        {temp.length > 0 && <SideLabel>{allTemp ? "Temporary folders" : "Temporary · active"}</SideLabel>}
        {shownTemp.map((p) => (
          <FolderLink key={p.id} p={p} sessions={sessions} />
        ))}
        {temp.length > tempActive.length && (
          <button className="h-[25px] flex-none rounded-md px-[9px] text-left text-[12px] text-ink-3 hover:bg-sunk" aria-expanded={allTemp} onClick={() => setAllTemp(!allTemp)}>
            {allTemp ? "Show only folders with agents" : `+ ${temp.length - tempActive.length} more temporary ${temp.length - tempActive.length === 1 ? "folder" : "folders"}`}
          </button>
        )}
      </Group>
      </nav>
      </div>
      <div className="flex-none border-t border-line-2 px-2 pt-1.5 pb-2.5">
      <FinishSetup />
      <div className="flex items-center gap-1">
        <div className="min-w-0 flex-1">
          <NavItem to="/settings" icon="sliders">
            Settings
          </NavItem>
        </div>
        <button type="button" className="flex size-[31px] flex-none items-center justify-center rounded-[7px] text-ink-2 hover:bg-sunk" aria-label="Dark theme" aria-pressed={theme === "dark"} title={`Switch to ${theme === "light" ? "dark" : "light"} theme on this device`} onClick={toggle}>
          <Icon name={theme === "dark" ? "moon" : "sun"} size={15} />
        </button>
      </div>
      </div>
    </aside>
  );
}
