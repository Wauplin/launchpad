import { useRouterState } from "@tanstack/react-router";
import { LayoutSwitch } from "@/features/agents/AgentsView";
import { LaunchButton } from "@/features/launch/LaunchButton";
import { useProjects } from "@/lib/queries";

export function TopBar() {
  const path = useRouterState({ select: (s) => s.location.pathname });
  const projects = useProjects().data ?? [];
  const projectId = path.match(/^\/projects\/([^/]+)/)?.[1];
  const project = projects.find((p) => p.id === projectId);
  const title = path === "/" ? "Inbox" : path.startsWith("/agents") ? "Agents" : path.startsWith("/settings") ? "Settings" : path.startsWith("/projects") ? "Projects" : "";
  return (
    <header className="flex h-[46px] flex-none items-center gap-3.5 border-b border-line bg-chrome pr-2.5 pl-3.5">
      <div className="flex items-center gap-2 font-bold tracking-[0.01em]">
        <span className="grid size-[22px] place-items-center rounded-md bg-teal">
          <svg viewBox="0 0 12 12" width="12" height="12">
            <path d="M6 1.5 10.5 10H1.5z" fill="#fff" />
          </svg>
        </span>
        Launchpad
      </div>
      <div className="flex items-center gap-1.5 text-ink-2">
        {project ? (
          <>
            Projects <span className="text-ink-3">/</span> <b className="font-semibold text-ink">{project.name}</b>
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
    </header>
  );
}
