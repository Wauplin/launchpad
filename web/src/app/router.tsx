import { createRootRoute, createRoute, createRouter, lazyRouteComponent } from "@tanstack/react-router";
import { AgentsView } from "@/features/agents/AgentsView";
import { CodeView, validateCodeSearch } from "@/features/code/CodeView";
import { InboxView } from "@/features/inbox/InboxView";
import { ProjectPage } from "@/features/projects/ProjectPage";
import { ProjectsView } from "@/features/projects/ProjectsView";
import { DevicesView } from "@/features/settings/DevicesView";
import { SettingsView } from "@/features/settings/SettingsView";
import { HarnessesView } from "@/features/settings/HarnessesView";
import { WorktreesView } from "@/features/settings/WorktreesView";
import { ItemView } from "@/features/github/ItemView";
import { PredraftView } from "@/features/predrafts/PredraftView";
import { PublishView } from "@/features/publish/PublishView";
import { WorkspacesView } from "@/features/workspaces/Workspaces";
import { Shell } from "./Shell";

const root = createRootRoute({ component: Shell });

const routeTree = root.addChildren([
  createRoute({ getParentRoute: () => root, path: "/predrafts/$id", component: PredraftView }),
  createRoute({
    getParentRoute: () => root,
    path: "/github/$owner/$repo/$number",
    component: ItemView,
    // `draft`: the agent suggestion to expand and scroll to.
    validateSearch: (s: Record<string, unknown>): { draft?: string } => (typeof s.draft === "string" || typeof s.draft === "number" ? { draft: String(s.draft) } : {}),
  }),
  createRoute({ getParentRoute: () => root, path: "/publish", component: PublishView }),
  createRoute({ getParentRoute: () => root, path: "/", component: InboxView }),
  createRoute({
    getParentRoute: () => root,
    path: "/agents",
    component: AgentsView,
    validateSearch: (s: Record<string, unknown>): { session?: string } => (typeof s.session === "string" ? { session: s.session } : {}),
  }),
  createRoute({ getParentRoute: () => root, path: "/projects", component: ProjectsView }),
  createRoute({ getParentRoute: () => root, path: "/projects/$id", component: ProjectPage }),
  createRoute({ getParentRoute: () => root, path: "/projects/$id/code", component: CodeView, validateSearch: validateCodeSearch }),
  createRoute({ getParentRoute: () => root, path: "/settings", component: SettingsView }),
  createRoute({ getParentRoute: () => root, path: "/settings/workspaces", component: WorkspacesView }),
  createRoute({ getParentRoute: () => root, path: "/settings/harnesses", component: HarnessesView }),
  createRoute({ getParentRoute: () => root, path: "/settings/worktrees", component: WorktreesView }),
  createRoute({ getParentRoute: () => root, path: "/settings/devices", component: DevicesView }),
  // Spike 4 benchmark page, not linked from the UI.
  createRoute({
    getParentRoute: () => root,
    path: "/dev/perf",
    component: lazyRouteComponent(() => import("@/features/dev/PerfView"), "PerfView"),
  }),
]);

export const router = createRouter({ routeTree, defaultPreload: "intent" });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
