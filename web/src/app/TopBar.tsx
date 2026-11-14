import { useQuery } from "@tanstack/react-query";
import { Link, useRouterState } from "@tanstack/react-router";
import { Fragment } from "react";
import type { Predraft } from "@/gen/Predraft";
import type { Project } from "@/gen/Project";
import logo from "@/assets/launchpad.png";
import { CommandField } from "./CommandField";
import { useProjects } from "@/lib/queries";
import { useBots } from "@/lib/bots";
import type { Bot } from "@/gen/Bot";
import type { Channel } from "@/gen/Channel";
import { isDemo } from "@/demo/mode";
import { DemoChip } from "@/onboarding/DemoChip";
import { SETTINGS_NAV, WORKSPACE_NAV } from "@/features/settings/nav";

type Crumb = { label: string; to?: string; params?: Record<string, string> };

// Settings page names by route segment, from the Settings nav (light: no Settings module).
const SETTINGS: Record<string, string> = {
  ...Object.fromEntries([...WORKSPACE_NAV, ...SETTINGS_NAV.flatMap((g) => g.items)].map((e) => [e.to.replace(/^\/settings\/?/, ""), e.label])),
};

/** The breadcrumb of a route: every segment but the last is a link. A top-level page (Today, Agents, Folders,
 * Setup) names itself in its page header, so it has no crumb; a detail page shows its parents. */
function routeCrumbs(path: string, projects: Project[], bots: Bot[], channels: Channel[]): Crumb[] | "predraft" {
  if (/^\/predrafts\/[^/]+/.test(path)) return "predraft";
  const settings = path.match(/^\/settings\/?([^/]*)/);
  if (settings) return [{ label: "Settings", to: "/settings" }, { label: SETTINGS[settings[1]] ?? "Settings" }];
  const project = path.match(/^\/projects\/([^/]+)(\/code)?/);
  if (project) {
    const p = projects.find((x) => x.id === project[1]);
    const folder = { label: p?.name ?? "…", to: "/projects/$id", params: { id: project[1] } };
    return project[2] ? [{ label: "Folders", to: "/projects" }, folder, { label: "Files" }] : [{ label: "Folders", to: "/projects" }, { label: folder.label }];
  }
  const bot = path.match(/^\/bots\/([^/]+)/);
  if (bot) return [{ label: "Bots", to: "/bots" }, { label: bots.find((b) => b.id === bot[1])?.settings.name ?? "…" }];
  // A bot channel or DM lives in the Bots area.
  const channel = path.match(/^\/channels\/([^/]+)/);
  if (channel) return [{ label: "Bots", to: "/bots" }, { label: channels.find((c) => c.id === channel[1])?.title ?? "…" }];
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

/** An embedding host (the Hugging Face Hub page of a Space) can cover the top-right corner of the page with its own UI.
 * `useHubPillInset` (Shell.tsx) sets `--lp-host-inset-right` and `--lp-host-inset-top` (unzoomed pixels) on the root element; the bar keeps its buttons out of that area. */
const HOST_INSET = {
  paddingRight: "max(10px, calc(var(--lp-host-inset-right, 0px) / var(--lp-zoom, 1)))",
  minHeight: "max(46px, calc(var(--lp-host-inset-top, 0px) / var(--lp-zoom, 1)))",
};

export function TopBar() {
  const path = useRouterState({ select: (s) => s.location.pathname });
  const projects = useProjects().data ?? [];
  const bots = useBots().data ?? [];
  // A channel's title from the open channel page or the sidebar's list (read from the cache, never fetched here).
  const channelId = path.match(/^\/channels\/([^/]+)/)?.[1] ?? "";
  const listed = useQuery<Channel[]>({ queryKey: ["channels"], enabled: false }).data ?? [];
  const opened = useQuery<{ channel: Channel }>({ queryKey: ["channel", channelId], enabled: false }).data?.channel;
  const channels = opened ? [opened] : listed;
  const crumbs = routeCrumbs(path, projects, bots, channels);
  const predraftId = path.match(/^\/predrafts\/([^/]+)/)?.[1];
  return (
    <header className="flex flex-none flex-wrap items-center gap-x-3.5 gap-y-1.5 border-b border-line bg-chrome py-1.5 pl-3.5" style={HOST_INSET}>
      <div className="flex shrink-0 items-center gap-2 font-bold tracking-[0.01em]">
        <img src={logo} alt="" className="size-[22px]" />
        Launchpad
      </div>
      <nav aria-label="Breadcrumb" className="flex min-w-0 shrink items-center gap-1.5 whitespace-nowrap text-ink-2">
        {crumbs === "predraft" && predraftId ? <PredraftCrumb id={predraftId} /> : Array.isArray(crumbs) && <Crumbs list={crumbs} />}
      </nav>
      <span className="ml-auto flex min-w-0 flex-1 items-center justify-end gap-2">
        {isDemo && <DemoChip />}
        <CommandField />
      </span>
    </header>
  );
}

function PredraftCrumb({ id }: { id: string }) {
  const projects = useProjects().data ?? [];
  const p = useQuery<{ predraft: Predraft }>({ queryKey: ["predrafts", id], enabled: false }).data?.predraft;
  const project = p && projects.find((x) => x.id === p.project_id);
  return <Crumbs list={[{ label: "Folders", to: "/projects" }, ...(project ? [{ label: project.name, to: "/projects/$id", params: { id: project.id } }] : []), { label: p?.title ?? "…" }]} />;
}
