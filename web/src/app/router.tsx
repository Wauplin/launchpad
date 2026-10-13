import { createRootRoute, createRoute, createRouter, lazyRouteComponent, redirect, type RouteComponent } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { validateCodeSearch } from "@/features/code/store";
import { isPhone } from "@/lib/phone";

/** The shell and the two first pages, loaded with it (app/desktop.ts or mobile/entry.ts). */
export type Entry = { Root: () => ReactNode; AgentsView: RouteComponent; InboxView: RouteComponent };

// Both shells (desktop `Shell`, phone `MobileShell`) share these routes. The desktop entry bundles Inbox
// and Agents with its shell; every other page is a separate chunk loaded on first visit (or on hover/focus
// of a link, `defaultPreload: "intent"`), so the first paint downloads less.
export function makeRouter({ Root, AgentsView, InboxView }: Entry) {
const root = createRootRoute({ component: Root });

const routeTree = root.addChildren([
  // Phone pages. The desktop shows the same information in Agents (dashboard) and To publish.
  createRoute({ getParentRoute: () => root, path: "/now", component: lazyRouteComponent(() => import("@/mobile/NowView"), "NowView"), beforeLoad: () => { if (!isPhone) throw redirect({ to: "/agents" }); } }),
  createRoute({ getParentRoute: () => root, path: "/review", component: lazyRouteComponent(() => import("@/features/review/ReviewView"), "ReviewView") }),
  createRoute({ getParentRoute: () => root, path: "/terminals", component: lazyRouteComponent(() => import("@/mobile/TerminalsView"), "TerminalsView") }),
  createRoute({ getParentRoute: () => root, path: "/inbox", component: InboxView }),
  createRoute({ getParentRoute: () => root, path: "/predrafts/$id", component: lazyRouteComponent(() => import("@/features/predrafts/PredraftView"), "PredraftView") }),
  createRoute({
    getParentRoute: () => root,
    path: "/github/$owner/$repo/$number",
    component: lazyRouteComponent(() => import("@/features/github/ItemView"), "ItemView"),
    // `draft`: the agent suggestion to expand and scroll to.
    validateSearch: (s: Record<string, unknown>): { draft?: string } => (typeof s.draft === "string" || typeof s.draft === "number" ? { draft: String(s.draft) } : {}),
  }),
  createRoute({ getParentRoute: () => root, path: "/publish", component: lazyRouteComponent(() => import("@/features/publish/PublishView"), "PublishView") }),
  // The phone opens on Now; its Inbox tab is /inbox.
  createRoute({ getParentRoute: () => root, path: "/", component: InboxView, beforeLoad: () => { if (isPhone) throw redirect({ to: "/now" }); } }),
  createRoute({
    getParentRoute: () => root,
    path: "/agents",
    component: AgentsView,
    // `view=changes` opens the session's Changes view (Review tab links).
    validateSearch: (s: Record<string, unknown>): { session?: string; view?: "changes" } => ({ ...(typeof s.session === "string" ? { session: s.session } : {}), ...(s.view === "changes" ? { view: "changes" as const } : {}) }),
  }),
  createRoute({ getParentRoute: () => root, path: "/projects", component: lazyRouteComponent(() => import("@/features/projects/ProjectsView"), "ProjectsView") }),
  createRoute({ getParentRoute: () => root, path: "/projects/$id", component: lazyRouteComponent(() => import("@/features/projects/ProjectPage"), "ProjectPage") }),
  createRoute({ getParentRoute: () => root, path: "/projects/$id/code", component: lazyRouteComponent(() => import("@/features/code/CodeView"), "CodeView"), validateSearch: validateCodeSearch }),
  createRoute({ getParentRoute: () => root, path: "/settings", component: lazyRouteComponent(() => import("@/features/settings/SettingsView"), "SettingsView") }),
  createRoute({ getParentRoute: () => root, path: "/settings/workspaces", component: lazyRouteComponent(() => import("@/features/workspaces/Workspaces"), "WorkspacesView") }),
  createRoute({ getParentRoute: () => root, path: "/settings/harnesses", component: lazyRouteComponent(() => import("@/features/settings/HarnessesView"), "HarnessesView") }),
  createRoute({ getParentRoute: () => root, path: "/settings/worktrees", component: lazyRouteComponent(() => import("@/features/settings/WorktreesView"), "WorktreesView") }),
  createRoute({ getParentRoute: () => root, path: "/settings/devices", component: lazyRouteComponent(() => import("@/features/settings/DevicesView"), "DevicesView") }),
  // Spike 4 benchmark page, not linked from the UI.
  createRoute({
    getParentRoute: () => root,
    path: "/dev/perf",
    component: lazyRouteComponent(() => import("@/features/dev/PerfView"), "PerfView"),
  }),
]);

/** Neutral placeholder in the main area while a page chunk downloads on a slow network. */
function PagePending() {
  return <div className="grid h-full place-items-center text-ink-3" role="status">Loading…</div>;
}

return createRouter({ routeTree, defaultPreload: "intent", defaultPendingComponent: PagePending });
}

declare module "@tanstack/react-router" {
  interface Register {
    router: ReturnType<typeof makeRouter>;
  }
}
