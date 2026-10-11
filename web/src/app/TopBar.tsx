import { useQuery } from "@tanstack/react-query";
import { Link, useRouterState } from "@tanstack/react-router";
import type { Predraft } from "@/gen/Predraft";
import type { Item } from "@/features/github/types";
import { itemKey } from "@/features/github/lib";
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
  // Inbox and Agents name themselves in the page; the breadcrumb would only repeat it.
  const title = path.startsWith("/settings") ? "Settings" : path.startsWith("/projects") ? "Projects" : "";
  const gh = path.match(/^\/github\/([^/]+)\/([^/]+)\/(\d+)/);
  const predraftId = path.match(/^\/predrafts\/([^/]+)/)?.[1];
  return (
    <header className="flex min-h-[46px] flex-none flex-wrap items-center gap-x-3.5 gap-y-1.5 border-b border-line bg-chrome py-1.5 pr-2.5 pl-3.5">
      <div className="flex shrink-0 items-center gap-2 font-bold tracking-[0.01em]">
        <img src={logo} alt="" className="size-[22px]" />
        Launchpad
      </div>
      <WorkspaceSwitcher />
      <div className="flex shrink-0 items-center gap-1.5 whitespace-nowrap text-ink-2">
        {gh ? (
          <ItemCrumb repo={`${gh[1]}/${gh[2]}`} number={Number(gh[3])} />
        ) : predraftId ? (
          <PredraftCrumb id={predraftId} />
        ) : project ? (
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
        ) : path === "/settings/harnesses" ? (
          <>
            Settings {sep} <b className="font-semibold text-ink">Harnesses</b>
          </>
        ) : path === "/settings/worktrees" ? (
          <>
            Settings {sep} <b className="font-semibold text-ink">Worktrees</b>
          </>
        ) : (
          <b className="font-semibold text-ink">{title}</b>
        )}
      </div>
      <span className="ml-auto flex min-w-0 shrink items-center gap-2">
        {/* The minimized composer renders its draft pill here (portal). */}
        <span id="draft-pill-slot" className="flex min-w-0 items-center" />
        <LaunchButton ctx={project ? { projectId: project.id } : undefined} />
      </span>
    </header>
  );
}

/** Reads the item the page already cached; it never fetches by itself. */
function ItemCrumb({ repo, number }: { repo: string; number: number }) {
  const projects = useProjects().data ?? [];
  const item = useQuery<Item>({ queryKey: itemKey(repo, number), enabled: false }).data;
  const lower = repo.toLowerCase();
  const project = projects.find((p) => p.gh_repo?.toLowerCase() === lower) ?? projects.find((p) => p.gh_upstream?.toLowerCase() === lower);
  return (
    <>
      {project ? <Link to="/projects/$id" params={{ id: project.id }} className="text-ink-2 no-underline hover:underline">{project.name}</Link> : repo} {sep}{" "}
      {item ? (item.__typename === "PullRequest" ? "Pull requests" : "Issues") : "Items"} {sep} <b className="font-semibold text-ink">#{number}</b>
    </>
  );
}

function PredraftCrumb({ id }: { id: string }) {
  const projects = useProjects().data ?? [];
  const p = useQuery<{ predraft: Predraft }>({ queryKey: ["predrafts", id], enabled: false }).data?.predraft;
  const project = p && projects.find((x) => x.id === p.project_id);
  return (
    <>
      {project && <><Link to="/projects/$id" params={{ id: project.id }} className="text-ink-2 no-underline hover:underline">{project.name}</Link> {sep} </>}
      Pre-draft PRs {sep} <b className="max-w-[420px] truncate font-semibold text-ink">{p?.title ?? "…"}</b>
    </>
  );
}
