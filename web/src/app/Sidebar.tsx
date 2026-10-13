import { useInboxNeeds } from "@/features/inbox/InboxView";
import { usePublishItems } from "@/features/publish/items";
import { useScope, useWorkspaces } from "@/lib/workspace";
import { Link } from "@tanstack/react-router";
import { useState, type ReactNode } from "react";
import type { Project } from "@/gen/Project";
import { isRunning, isQuotaPaused, needsYou } from "@/lib/sessionStatus";
import type { Session } from "@/gen/Session";
import { Icon, type IconName } from "@/components/Icon";
import { formatAge } from "@/lib/format";
import { useProjects, useSessions } from "@/lib/queries";

function NavItem({ to, icon, children, end }: { to: string; icon: IconName; children: ReactNode; end?: ReactNode }) {
  return (
    <Link
      to={to}
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

export function SideLabel({ children }: { children: ReactNode }) {
  return <div className="flex items-center gap-1.5 px-[9px] pt-3.5 pb-1 seclabel">{children}</div>;
}

const inProject = (p: Project, s: Session) => s.project_id === p.id || s.cwd === p.path || s.cwd.startsWith(p.path + "/");
/** Needs you, quota paused or running: the states the sidebar marks. */
const active = (s: Session) => needsYou(s) || isQuotaPaused(s) || isRunning(s);

function RunDot({ p, sessions }: { p: Project; sessions: Session[] }) {
  const active = sessions.filter((s) => inProject(p, s));
  return <span className="flex items-center gap-1.5">
    {active.some(needsYou) && <span className="size-[7px] rounded-full bg-attn" title="Needs you" />}
    {active.some(isQuotaPaused) && <span className="size-[7px] rounded-full border border-attn" title="Quota paused" />}
    {active.some(isRunning) && <span className="lp-spin" title="Running" />}
  </span>;
}

function ProjectLink({ p, sessions }: { p: Project; sessions: Session[] }) {
  return (
    <Link
      to="/projects/$id"
      params={{ id: p.id }}
      className={`flex h-[27px] items-center gap-2 rounded-md px-[9px] no-underline hover:bg-sunk ${p.is_temp ? "text-[12px] text-ink-2" : "text-[12.5px] text-ink"}`}
      activeProps={{ className: "!bg-paper shadow-[inset_0_0_0_1px_var(--color-line)]" }}
      title={p.is_temp ? `${p.name} · created ${formatAge(p.created_at)} ago` : undefined}
    >
      <span className="min-w-0 truncate">{p.is_temp ? p.name.replace(/^tmp-\d{4}-/, "") || p.name : p.name}</span>
      <span className="ml-auto">
        <RunDot p={p} sessions={sessions} />
      </span>
    </Link>
  );
}

export function Sidebar() {
  const needs = useInboxNeeds();
  const toPublish = usePublishItems().length;
  const scope = useScope();
  const all = useProjects().data ?? [];
  // A workspace lists its projects; All workspaces combines them with explicit global pins.
  const workspaceProjects = new Set((useWorkspaces().data ?? []).flatMap((w) => w.project_ids));
  const projects = scope.workspace ? scope.projects : all;
  const sessions = (useSessions().data ?? []).filter(scope.hasSession);
  const [allTemp, setAllTemp] = useState(false);
  const waiting = sessions.filter(needsYou).length;
  const running = sessions.filter(isRunning).length;
  const quotaPaused = sessions.filter(isQuotaPaused).length;
  const pinned = projects.filter((p) => (scope.workspace || p.pinned || workspaceProjects.has(p.id)) && !p.is_temp).sort((a, b) => a.name.localeCompare(b.name));
  const temp = projects.filter((p) => p.is_temp).sort((a, b) => b.created_at - a.created_at);
  // Temporary folders: only those with agents that need you, run or wait for quota, unless expanded.
  const tempActive = temp.filter((p) => sessions.some((s) => inProject(p, s) && active(s)));
  const shownTemp = allTemp ? temp : tempActive;
  return (
    <aside className="flex w-[200px] flex-none flex-col gap-px overflow-y-auto border-r border-line bg-chrome px-2 py-2.5">
      <NavItem to="/" icon="inbox" end={needs > 0 ? <span className="rounded-full bg-attn-soft px-1.5 text-[11px] font-semibold text-attn tabular-nums" title={`${needs} unread review requests, mentions and assignments`}>{needs}</span> : undefined}>
        Inbox
      </NavItem>
      <NavItem to="/agents" icon="agents">
        <span className="min-w-0 flex-1">
          <span className="block">Agents</span>
          {(waiting > 0 || quotaPaused > 0 || running > 0) && (
            <span className="mt-1 flex flex-wrap items-center gap-x-1.5 gap-y-1 text-[11px] font-semibold tabular-nums">
              {waiting > 0 && <span className="rounded-full bg-attn-soft px-1.5 whitespace-nowrap text-attn" title={`${waiting} need you: questions and unseen Done, Failed or Interrupted sessions`}>{waiting} waiting</span>}
              {quotaPaused > 0 && <span className="rounded-full bg-attn-soft px-1.5 whitespace-nowrap text-attn" title={`${quotaPaused} quota paused`}>{quotaPaused} paused</span>}
              {running > 0 && <span className="flex items-center gap-1 whitespace-nowrap text-work" title={`${running} running`}><span className="lp-spin" />{running} running</span>}
            </span>
          )}
        </span>
      </NavItem>
      <NavItem to="/publish" icon="pr-local" end={toPublish > 0 ? <span className="text-[11px] font-semibold text-ink-3 tabular-nums" title={`${toPublish} agent drafts and pre-draft PRs wait for your click`}>{toPublish}</span> : undefined}>
        To publish
      </NavItem>
      <NavItem to="/projects" icon="folder">
        Projects
      </NavItem>
      {scope.workspace ? <SideLabel>{scope.workspace.name}</SideLabel> : pinned.length > 0 && <SideLabel>Pinned</SideLabel>}
      {scope.workspace && pinned.length === 0 && <Link to="/settings/workspaces" className="px-[9px] py-1 text-xs text-ink-3">No project yet: pick some</Link>}
      {pinned.map((p) => (
        <ProjectLink key={p.id} p={p} sessions={sessions} />
      ))}
      {temp.length > 0 && <SideLabel>{allTemp ? "Temporary" : "Temporary · with agents"}</SideLabel>}
      {shownTemp.map((p) => (
        <ProjectLink key={p.id} p={p} sessions={sessions} />
      ))}
      {temp.length > tempActive.length && (
        <button className="h-[25px] rounded-md px-[9px] text-left text-[12px] text-ink-3 hover:bg-sunk" aria-expanded={allTemp} onClick={() => setAllTemp(!allTemp)}>
          {allTemp ? "Show only folders with agents" : `+ ${temp.length - tempActive.length} more temporary ${temp.length - tempActive.length === 1 ? "folder" : "folders"}`}
        </button>
      )}
      <div className="mt-auto pt-2">
        <NavItem to="/settings" icon="sliders">
          Settings
        </NavItem>
      </div>
    </aside>
  );
}
