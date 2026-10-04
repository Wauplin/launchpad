import { useRouterState } from "@tanstack/react-router";
import logo from "@/assets/launchpad.png";
import { LayoutSwitch } from "@/features/agents/AgentsView";
import { LaunchButton } from "@/features/launch/LaunchButton";
import { useProjects } from "@/lib/queries";
import { WorkspaceSwitcher } from "@/features/workspaces/Workspaces";

export function TopBar() {
  const path = useRouterState({ select: (s) => s.location.pathname });
  const projects = useProjects().data ?? [];
  const projectId = path.match(/^\/projects\/([^/]+)/)?.[1];
  const project = projects.find((p) => p.id === projectId);
  const title = path === "/" ? "Inbox" : path.startsWith("/agents") ? "Agents" : path.startsWith("/settings") ? "Settings" : path.startsWith("/workspaces") ? "Workspaces" : path.startsWith("/projects") ? "Projects" : "";
  return (
    <header className="flex-none border-b border-line bg-chrome">
      <div className="flex h-[46px] items-center gap-3.5 pr-2.5 pl-3.5">
        <div className="flex items-center gap-2 font-bold tracking-[0.01em]">
          <img src={logo} alt="" className="size-[22px]" />
          Launchpad
        </div>
        <div className="flex items-center gap-1.5 text-ink-2">
          {project ? (
            <>
              Projects <span className="text-ink-3">/</span>{" "}
              {path.endsWith("/code") ? (
                <>
                  {project.name} <span className="text-ink-3">/</span> <b className="font-semibold text-ink">Code</b>
                </>
              ) : (
                <b className="font-semibold text-ink">{project.name}</b>
              )}
            </>
          ) : (
            <b className="font-semibold text-ink">{title}</b>
          )}
        </div>
        {path.startsWith("/agents") && (
          <span className="ml-[18px]">
            <LayoutSwitch />
          </span>
        )}
        <span className="ml-auto">
          <LaunchButton ctx={project ? { projectId: project.id } : undefined} />
        </span>
      </div>
      <WorkspaceSwitcher />
    </header>
  );
}
