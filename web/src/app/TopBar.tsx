import { useRouterState } from "@tanstack/react-router";
import logo from "@/assets/launchpad.png";
import { LaunchButton } from "@/features/launch/LaunchButton";
import { useProjects } from "@/lib/queries";
import { WorkspaceSwitcher } from "@/features/workspaces/Workspaces";

const sep = <span className="text-ink-3">/</span>;

export function TopBar() {
  const path = useRouterState({ select: (s) => s.location.pathname });
  const projects = useProjects().data ?? [];
  const projectId = path.match(/^\/projects\/([^/]+)/)?.[1];
  const project = projects.find((p) => p.id === projectId);
  const title = path === "/" ? "Inbox" : path.startsWith("/agents") ? "Agents" : path.startsWith("/settings") ? "Settings" : path.startsWith("/projects") ? "Projects" : "";
  return (
    <header className="flex min-h-[46px] flex-none flex-wrap items-center gap-x-3.5 gap-y-1.5 border-b border-line bg-chrome py-1.5 pr-2.5 pl-3.5">
      <div className="flex shrink-0 items-center gap-2 font-bold tracking-[0.01em]">
        <img src={logo} alt="" className="size-[22px]" />
        Launchpad
      </div>
      <WorkspaceSwitcher />
      <div className="flex shrink-0 items-center gap-1.5 whitespace-nowrap text-ink-2">
        {project ? (
          <>
            Projects {sep}{" "}
            {path.endsWith("/code") ? (
              <>
                {project.name} {sep} <b className="font-semibold text-ink">Code</b>
              </>
            ) : (
              <b className="font-semibold text-ink">{project.name}</b>
            )}
          </>
        ) : path === "/settings/workspaces" ? (
          <>
            Settings {sep} <b className="font-semibold text-ink">Workspaces</b>
          </>
        ) : (
          <b className="font-semibold text-ink">{title}</b>
        )}
      </div>
      <span className="ml-auto shrink-0">
        <LaunchButton ctx={project ? { projectId: project.id } : undefined} />
      </span>
    </header>
  );
}
