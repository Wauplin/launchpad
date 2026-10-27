import { useQuery } from "@tanstack/react-query";
import { Link, useRouterState } from "@tanstack/react-router";
import { Fragment } from "react";
import type { Predraft } from "@/gen/Predraft";
import type { Project } from "@/gen/Project";
import type { Item } from "@/features/github/types";
import { itemKey } from "@/features/github/lib";
import logo from "@/assets/launchpad.png";
import { TopLaunchButton } from "@/features/launch/LaunchButton";
import { useProjects } from "@/lib/queries";
import { WorkspaceSwitcher } from "@/features/workspaces/Switcher";

type Crumb = { label: string; to?: string; params?: Record<string, string> };

// Settings page names, kept here so the top bar does not load the Settings module.
const SETTINGS: Record<string, string> = {
  "": "Appearance",
  notifications: "Notifications",
  profiles: "Profiles",
  presets: "Presets",
  usage: "Usage",
  hosts: "Machines",
  devices: "Devices",
  github: "GitHub",
  huggingface: "Hugging Face",
  worktrees: "Maintenance",
};

/** The breadcrumb of a route: every segment but the last is a link. Inbox, Now and Agents name themselves. */
function routeCrumbs(path: string, projects: Project[]): Crumb[] | "item" | "predraft" {
  if (/^\/github\/[^/]+\/[^/]+\/\d+/.test(path)) return "item";
  if (/^\/predrafts\/[^/]+/.test(path)) return "predraft";
  const settings = path.match(/^\/settings\/?([^/]*)/);
  if (settings) return [{ label: "Settings", to: "/settings" }, { label: SETTINGS[settings[1]] ?? "Settings" }];
  const project = path.match(/^\/projects\/([^/]+)(\/code)?/);
  if (project) {
    const p = projects.find((x) => x.id === project[1]);
    const folder = { label: p?.name ?? "…", to: "/projects/$id", params: { id: project[1] } };
    return project[2] ? [{ label: "Folders", to: "/projects" }, folder, { label: "Files" }] : [{ label: "Folders", to: "/projects" }, { label: folder.label }];
  }
  if (path.startsWith("/projects")) return [{ label: "Folders" }];
  if (path.startsWith("/review")) return [{ label: "Review" }];
  if (path.startsWith("/ask")) return [{ label: "Ask Launchpad" }];
  if (path.startsWith("/terminals")) return [{ label: "Terminals" }];
  return [];
}

function Crumbs({ list }: { list: Crumb[] }) {
  return (
    <>
      {list.map((c, i) => (
        <Fragment key={i}>
          {i > 0 && <span className="text-ink-3">/</span>}
          {i === list.length - 1 ? <b className="max-w-[420px] truncate font-semibold text-ink">{c.label}</b> : <Link to={c.to!} params={c.params as never} className="truncate text-ink-2 no-underline hover:text-ink hover:underline">{c.label}</Link>}
        </Fragment>
      ))}
    </>
  );
}

export function TopBar() {
  const path = useRouterState({ select: (s) => s.location.pathname });
  const projects = useProjects().data ?? [];
  const projectId = path.match(/^\/projects\/([^/]+)/)?.[1];
  const project = projects.find((p) => p.id === projectId);
  const crumbs = routeCrumbs(path, projects);
  const gh = path.match(/^\/github\/([^/]+)\/([^/]+)\/(\d+)/);
  const predraftId = path.match(/^\/predrafts\/([^/]+)/)?.[1];
  return (
    <header className="flex min-h-[46px] flex-none flex-wrap items-center gap-x-3.5 gap-y-1.5 border-b border-line bg-chrome py-1.5 pr-2.5 pl-3.5">
      <div className="flex shrink-0 items-center gap-2 font-bold tracking-[0.01em]">
        <img src={logo} alt="" className="size-[22px]" />
        Launchpad
      </div>
      <WorkspaceSwitcher />
      <nav aria-label="Breadcrumb" className="flex min-w-0 shrink items-center gap-1.5 whitespace-nowrap text-ink-2">
        {crumbs === "item" && gh ? <ItemCrumb repo={`${gh[1]}/${gh[2]}`} number={Number(gh[3])} /> : crumbs === "predraft" && predraftId ? <PredraftCrumb id={predraftId} /> : Array.isArray(crumbs) && <Crumbs list={crumbs} />}
      </nav>
      <span className="ml-auto flex min-w-0 shrink-0 items-center gap-2">
        <TopLaunchButton ctx={project ? { projectId: project.id } : undefined} />
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
  const kind = item ? (item.__typename === "PullRequest" ? "Pull requests" : "Issues") : "Items";
  const list: Crumb[] = project ? [{ label: project.name, to: "/projects/$id", params: { id: project.id } }, { label: kind, to: "/projects/$id", params: { id: project.id } }] : [{ label: repo, to: "/inbox" }, { label: kind, to: "/inbox" }];
  return <Crumbs list={[...list, { label: `#${number}` }]} />;
}

function PredraftCrumb({ id }: { id: string }) {
  const projects = useProjects().data ?? [];
  const p = useQuery<{ predraft: Predraft }>({ queryKey: ["predrafts", id], enabled: false }).data?.predraft;
  const project = p && projects.find((x) => x.id === p.project_id);
  return <Crumbs list={[...(project ? [{ label: project.name, to: "/projects/$id", params: { id: project.id } }] : []), { label: "Local PRs", to: "/review" }, { label: p?.title ?? "…" }]} />;
}
