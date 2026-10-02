import { createRootRoute, createRoute, createRouter } from "@tanstack/react-router";
import { AgentsView } from "@/features/agents/AgentsView";
import { InboxView } from "@/features/inbox/InboxView";
import { ProjectsView } from "@/features/projects/ProjectsView";
import { SettingsView } from "@/features/settings/SettingsView";
import { Shell } from "./Shell";

const root = createRootRoute({ component: Shell });

const routeTree = root.addChildren([
  createRoute({ getParentRoute: () => root, path: "/", component: InboxView }),
  createRoute({ getParentRoute: () => root, path: "/agents", component: AgentsView }),
  createRoute({ getParentRoute: () => root, path: "/projects", component: ProjectsView }),
  createRoute({ getParentRoute: () => root, path: "/settings", component: SettingsView }),
]);

export const router = createRouter({ routeTree, defaultPreload: "intent" });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
