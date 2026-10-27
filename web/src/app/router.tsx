import { createRootRoute, createRoute, createRouter, lazyRouteComponent, redirect, type RouteComponent, type RouterHistory } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { validateCodeSearch } from "@/features/code/store";
import { isPhone } from "@/lib/phone";

/** The shell and the two first pages, loaded with it (app/desktop.ts or mobile/entry.ts). */
export type Entry = { Root: () => ReactNode; AgentsView: RouteComponent; InboxView: RouteComponent };

// Both shells (desktop `Shell`, phone `MobileShell`) share these routes. The desktop entry bundles Inbox
// and Agents with its shell; every other page is a separate chunk loaded on first visit (or on hover/focus
// of a link, `defaultPreload: "intent"`), so the first paint downloads less.
export function makeRouter({ Root, AgentsView, InboxView }: Entry, history?: RouterHistory) {
const root = createRootRoute({ component: Root });

const routeTree = root.addChildren([
  // Phone pages. The desktop shows the same information in Agents (dashboard). Review is shared by both shells.
  createRoute({ getParentRoute: () => root, path: "/now", component: lazyRouteComponent(() => import("@/mobile/NowView"), "NowView"), beforeLoad: () => { if (!isPhone) throw redirect({ to: "/agents" }); } }),
  createRoute({ getParentRoute: () => root, path: "/review", component: lazyRouteComponent(() => import("@/features/review/ReviewView"), "ReviewView") }),
  createRoute({ getParentRoute: () => root, path: "/terminals", component: lazyRouteComponent(() => import("@/mobile/TerminalsView"), "TerminalsView") }),
  createRoute({ getParentRoute: () => root, path: "/inbox", component: InboxView }),
  createRoute({ getParentRoute: () => root, path: "/ask", component: lazyRouteComponent(() => import("@/features/ask/AskView"), "AskView"), validateSearch: (s: Record<string, unknown>): { q?: string } => (typeof s.q === "string" && s.q ? { q: s.q } : {}) }),
  createRoute({ getParentRoute: () => root, path: "/predrafts/$id", component: lazyRouteComponent(() => import("@/features/predrafts/PredraftView"), "PredraftView") }),
  createRoute({
    getParentRoute: () => root,
    path: "/github/$owner/$repo/$number",
    component: lazyRouteComponent(() => import("@/features/github/ItemView"), "ItemView"),
    // `draft`: the agent suggestion to expand and scroll to.
    validateSearch: (s: Record<string, unknown>): { draft?: string } => (typeof s.draft === "string" || typeof s.draft === "number" ? { draft: String(s.draft) } : {}),
  }),
  // The former To publish page is the To publish group of Review.
  createRoute({ getParentRoute: () => root, path: "/publish", beforeLoad: () => { throw redirect({ to: "/review" }); } }),
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
  // Workspaces are managed from the sidebar; Analytics became Usage.
  createRoute({ getParentRoute: () => root, path: "/settings/workspaces", beforeLoad: () => { throw redirect({ to: "/settings" }); } }),
  createRoute({ getParentRoute: () => root, path: "/settings/analytics", beforeLoad: () => { throw redirect({ to: "/settings/usage" }); } }),
  createRoute({ getParentRoute: () => root, path: "/settings/notifications", component: lazyRouteComponent(() => import("@/features/settings/DevicesView"), "NotificationsPage") }),
  createRoute({ getParentRoute: () => root, path: "/settings/presets", component: lazyRouteComponent(() => import("@/features/settings/SettingsView"), "PresetsView") }),
  createRoute({ getParentRoute: () => root, path: "/settings/github", component: lazyRouteComponent(() => import("@/features/settings/GitHubView"), "GitHubView") }),
  createRoute({ getParentRoute: () => root, path: "/settings/profiles", component: lazyRouteComponent(() => import("@/features/settings/ProfilesView"), "ProfilesView") }),
  createRoute({ getParentRoute: () => root, path: "/settings/usage", component: lazyRouteComponent(() => import("@/features/settings/AnalyticsView"), "AnalyticsView") }),
  createRoute({ getParentRoute: () => root, path: "/settings/worktrees", component: lazyRouteComponent(() => import("@/features/settings/WorktreesView"), "WorktreesView") }),
  createRoute({ getParentRoute: () => root, path: "/settings/hosts", component: lazyRouteComponent(() => import("@/features/settings/HostsView"), "HostsView") }),
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

return createRouter({ history, routeTree, defaultPreload: "intent", defaultPendingComponent: PagePending });
}

declare module "@tanstack/react-router" {
  interface Register {
    router: ReturnType<typeof makeRouter>;
  }
}
