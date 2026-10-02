import { createRootRoute, createRoute, createRouter, lazyRouteComponent } from "@tanstack/react-router";
import { AgentsView } from "@/features/agents/AgentsView";
import { InboxView } from "@/features/inbox/InboxView";
import { ProjectPage } from "@/features/projects/ProjectPage";
import { ProjectsView } from "@/features/projects/ProjectsView";
import { SettingsView } from "@/features/settings/SettingsView";
import { Shell } from "./Shell";

const root = createRootRoute({ component: Shell });

const routeTree = root.addChildren([
  createRoute({ getParentRoute: () => root, path: "/", component: InboxView }),
  createRoute({
    getParentRoute: () => root,
    path: "/agents",
    component: AgentsView,
    validateSearch: (s: Record<string, unknown>): { session?: string } => (typeof s.session === "string" ? { session: s.session } : {}),
  }),
  createRoute({ getParentRoute: () => root, path: "/projects", component: ProjectsView }),
  createRoute({ getParentRoute: () => root, path: "/projects/$id", component: ProjectPage }),
  createRoute({ getParentRoute: () => root, path: "/settings", component: SettingsView }),
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
