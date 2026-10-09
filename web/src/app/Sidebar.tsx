import { useInboxThreads } from "@/features/inbox/InboxView";
import { useScope } from "@/lib/workspace";
import { Link } from "@tanstack/react-router";
import type { ReactNode } from "react";
import type { Project } from "@/gen/Project";
import { isDone, isRunning } from "@/lib/sessionStatus";
import type { Session } from "@/gen/Session";
import { Icon, type IconName } from "@/components/Icon";
import { formatAge } from "@/lib/format";
import { useProjects, useSessions } from "@/lib/queries";

function NavItem({ to, icon, children, end }: { to: string; icon: IconName; children: ReactNode; end?: ReactNode }) {
  return (
    <Link
      to={to}
      className="flex h-[31px] items-center gap-[9px] rounded-[7px] px-[9px] text-ink-2 no-underline hover:bg-sunk"
      activeProps={{ className: "!bg-paper !text-ink font-semibold shadow-[inset_0_0_0_1px_var(--color-line)]" }}
      activeOptions={{ exact: to === "/" }}
    >
      <Icon name={icon} />
      {children}
      {end && <span className="ml-auto">{end}</span>}
    </Link>
  );
}

export function SideLabel({ children }: { children: ReactNode }) {
  return <div className="flex items-center gap-1.5 px-[9px] pt-3.5 pb-1 seclabel">{children}</div>;
}

function RunDot({ p, sessions }: { p: Project; sessions: Session[] }) {
  const active = sessions.filter((s) => s.project_id === p.id || s.cwd === p.path || s.cwd.startsWith(p.path + "/"));
  return <span className="flex items-center gap-1.5">
    {active.some(isDone) && <span className="size-[7px] rounded-full bg-attn" title="Done: waiting for input" />}
    {active.some(isRunning) && <span className="lp-spin" title="Running" />}
  </span>;
}

function ProjectLink({ p, sessions }: { p: Project; sessions: Session[] }) {
  return (
    <Link
      to="/projects/$id"
      params={{ id: p.id }}
      className={`flex h-[27px] items-center gap-2 rounded-md px-[9px] text-ink no-underline hover:bg-sunk ${p.is_temp ? "font-mono text-[11.5px] text-ink-2" : "text-[12.5px]"}`}
      activeProps={{ className: "!bg-paper shadow-[inset_0_0_0_1px_var(--color-line)]" }}
    >
      <span className="min-w-0 truncate">{p.name}</span>
      {p.is_temp && <span className="ml-auto flex-none text-[11px] whitespace-nowrap text-ink-3">{formatAge(p.created_at)}</span>}
      <span className={p.is_temp ? "" : "ml-auto"}>
        <RunDot p={p} sessions={sessions} />
      </span>
    </Link>
  );
}

export function Sidebar() {
  const unread = useInboxThreads().threads.filter((t) => t.unread).length;
  const scope = useScope();
  const all = useProjects().data ?? [];
  // A workspace lists all its projects; without one, the pinned projects.
  const projects = scope.workspace ? scope.projects : all;
  const sessions = (useSessions().data ?? []).filter(scope.hasSession);
  const done = sessions.filter(isDone).length;
  const running = sessions.filter(isRunning).length;
  const pinned = projects.filter((p) => (scope.workspace || p.pinned) && !p.is_temp).sort((a, b) => a.name.localeCompare(b.name));
  const temp = projects.filter((p) => p.is_temp).sort((a, b) => b.created_at - a.created_at);
  return (
    <aside className="flex w-[200px] flex-none flex-col gap-px overflow-y-auto border-r border-line bg-chrome px-2 py-2.5">
      <NavItem to="/" icon="inbox" end={unread > 0 ? <span className="text-xs text-ink-3">{unread}</span> : undefined}>
        Inbox
      </NavItem>
      <NavItem
        to="/agents"
        icon="agents"
        end={
          <span className="flex items-center gap-2 text-[11px] font-semibold tabular-nums">
            <span className="rounded-full bg-attn-soft px-1.5 text-attn" title={`${done} Done: waiting for input`}>{done}</span>
            <span className="flex items-center gap-1 text-work" title={`${running} Running`}>
              {running > 0 && <span className="lp-spin" />}{running}
            </span>
          </span>
        }
      >
        Agents
      </NavItem>
      <NavItem to="/projects" icon="folder">
        Projects
      </NavItem>
      {scope.workspace ? <SideLabel>{scope.workspace.name}</SideLabel> : pinned.length > 0 && <SideLabel>Pinned</SideLabel>}
      {scope.workspace && pinned.length === 0 && <Link to="/settings/workspaces" className="px-[9px] py-1 text-xs text-ink-3">No project yet: pick some</Link>}
      {pinned.map((p) => (
        <ProjectLink key={p.id} p={p} sessions={sessions} />
      ))}
      {temp.length > 0 && <SideLabel>Temporary</SideLabel>}
      {temp.map((p) => (
        <ProjectLink key={p.id} p={p} sessions={sessions} />
      ))}
      <div className="mt-auto pt-2">
        <NavItem to="/settings" icon="sliders">
          Settings
        </NavItem>
      </div>
    </aside>
  );
}
