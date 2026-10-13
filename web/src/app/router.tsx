import { createRootRoute, createRoute, createRouter, lazyRouteComponent } from "@tanstack/react-router";
import { AgentsView } from "@/features/agents/AgentsView";
import { validateCodeSearch } from "@/features/code/store";
import { InboxView } from "@/features/inbox/InboxView";
import { Shell } from "./Shell";

// Inbox and Agents are in the main bundle; every other page is a separate chunk loaded on first
// visit (or on hover/focus of a link, `defaultPreload: "intent"`), so the first paint downloads less.
const root = createRootRoute({ component: Shell });

const routeTree = root.addChildren([
  createRoute({ getParentRoute: () => root, path: "/predrafts/$id", component: lazyRouteComponent(() => import("@/features/predrafts/PredraftView"), "PredraftView") }),
  createRoute({
    getParentRoute: () => root,
    path: "/github/$owner/$repo/$number",
    component: lazyRouteComponent(() => import("@/features/github/ItemView"), "ItemView"),
    // `draft`: the agent suggestion to expand and scroll to.
    validateSearch: (s: Record<string, unknown>): { draft?: string } => (typeof s.draft === "string" || typeof s.draft === "number" ? { draft: String(s.draft) } : {}),
  }),
  createRoute({ getParentRoute: () => root, path: "/publish", component: lazyRouteComponent(() => import("@/features/publish/PublishView"), "PublishView") }),
  createRoute({ getParentRoute: () => root, path: "/", component: InboxView }),
  createRoute({
    getParentRoute: () => root,
    path: "/agents",
    component: AgentsView,
    validateSearch: (s: Record<string, unknown>): { session?: string } => (typeof s.session === "string" ? { session: s.session } : {}),
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

export const router = createRouter({ routeTree, defaultPreload: "intent", defaultPendingComponent: PagePending });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
