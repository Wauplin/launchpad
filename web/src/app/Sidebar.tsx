import { useKnowledgeUnseen } from "@/lib/knowledge";
import { useInboxNeeds } from "@/features/inbox/data";
import { useTodayCount } from "@/features/today/data";
import { botInitial, botState, botThread, useBots } from "@/lib/bots";
import type { Bot } from "@/gen/Bot";
import { useScope } from "@/lib/workspace";
import { WorkspaceMenu } from "@/features/workspaces/WorkspaceMenu";
import { Link } from "@tanstack/react-router";
import { memo, type ReactNode } from "react";
import type { Project } from "@/gen/Project";
import { isRunning, isQuotaPaused, needsYou } from "@/lib/sessionStatus";
import type { Session } from "@/gen/Session";
import { Icon, type IconName } from "@/components/Icon";
import { toastError } from "@/components/Toast";
import { useSessions } from "@/lib/queries";
import { useTheme } from "@/stores/theme";
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

/** State dots of a set of sessions (Folders): needs you, quota paused, running. */
function RunDots({ sessions }: { sessions: Session[] }) {
  return <span className="flex items-center gap-1.5">
    {sessions.some(needsYou) && <span className="size-[7px] rounded-full bg-attn" title="Needs you" />}
    {sessions.some(isQuotaPaused) && <span className="size-[7px] rounded-full border border-attn" title="Quota paused" />}
    {sessions.some(isRunning) && <span className="lp-spin" title="Running" />}
  </span>;
}

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
  const projects = scope.projects;
  const sessions = (useSessions().data ?? []).filter(scope.hasSession);
  const running = sessions.filter(isRunning).length;
  const quotaPaused = sessions.filter(isQuotaPaused).length;
  return (
    <aside className="flex w-[200px] flex-none flex-col border-r border-line bg-chrome">
      {/* One scroll area for the groups; Settings stays at the bottom. */}
      <WorkspaceMenu />
      <div className="relative flex min-h-0 flex-1 flex-col gap-px overflow-y-auto px-2 pt-2.5 pb-2">
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
      <NavItem tour="sidebar.folders" to="/projects" icon="folder" end={<RunDots sessions={sessions.filter((s) => projects.some((p) => inProject(p, s)))} />}>
        Folders
      </NavItem>
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
