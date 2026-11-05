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
  createRoute({ getParentRoute: () => root, path: "/outbox", component: lazyRouteComponent(() => import("@/features/outbox/OutboxView"), "OutboxView") }),
  createRoute({ getParentRoute: () => root, path: "/proposals", component: lazyRouteComponent(() => import("@/features/proposals/ProposalsView"), "ProposalsView"), validateSearch: (s: Record<string, unknown>): { proposal?: string } => (typeof s.proposal === "string" && s.proposal ? { proposal: s.proposal } : {}) }),
  createRoute({ getParentRoute: () => root, path: "/bots", component: lazyRouteComponent(() => import("@/features/bots/BotsView"), "BotsView"), validateSearch: (s: Record<string, unknown>): { new?: boolean } => (s.new === true || s.new === "true" || s.new === 1 ? { new: true } : {}) }),
  createRoute({ getParentRoute: () => root, path: "/bots/$id", component: lazyRouteComponent(() => import("@/features/bots/BotsView"), "BotPage"), validateSearch: (s: Record<string, unknown>): { panel?: string } => (typeof s.panel === "string" && s.panel ? { panel: s.panel } : {}) }),
  createRoute({ getParentRoute: () => root, path: "/schedules", component: lazyRouteComponent(() => import("@/features/schedules/SchedulesView"), "SchedulesView"), validateSearch: (s: Record<string, unknown>): { tab?: "routines" | "launches"; routine?: string; launch?: string } => ({ ...(s.tab === "routines" || s.tab === "launches" ? { tab: s.tab } : {}), ...(typeof s.routine === "string" && s.routine ? { routine: s.routine } : {}), ...(typeof s.launch === "string" && s.launch ? { launch: s.launch } : {}) }) }),
  createRoute({ getParentRoute: () => root, path: "/predrafts/$id", component: lazyRouteComponent(() => import("@/features/predrafts/PredraftView"), "PredraftView") }),
  createRoute({
    getParentRoute: () => root,
    path: "/github/$owner/$repo/$number",
    component: lazyRouteComponent(() => import("@/features/github/ItemView"), "ItemView"),
    // `draft`: the agent suggestion to expand and scroll to.
    validateSearch: (s: Record<string, unknown>): { draft?: string } => (typeof s.draft === "string" || typeof s.draft === "number" ? { draft: String(s.draft) } : {}),
  }),
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
  createRoute({ getParentRoute: () => root, path: "/knowledge", component: lazyRouteComponent(() => import("@/features/knowledge/KnowledgeView"), "KnowledgeView") }),
  createRoute({ getParentRoute: () => root, path: "/setup", component: lazyRouteComponent(() => import("@/features/setup/SetupView"), "SetupView") }),
  createRoute({ getParentRoute: () => root, path: "/settings", component: lazyRouteComponent(() => import("@/features/settings/SettingsView"), "SettingsView") }),
  createRoute({ getParentRoute: () => root, path: "/settings/notifications", component: lazyRouteComponent(() => import("@/features/settings/SettingsView"), "NotificationsView") }),
  createRoute({ getParentRoute: () => root, path: "/settings/presets", component: lazyRouteComponent(() => import("@/features/settings/SettingsView"), "PresetsView") }),
  createRoute({ getParentRoute: () => root, path: "/settings/huggingface", component: lazyRouteComponent(() => import("@/features/settings/HuggingFaceView"), "HuggingFaceView") }),
  createRoute({ getParentRoute: () => root, path: "/settings/github", component: lazyRouteComponent(() => import("@/features/settings/GitHubView"), "GitHubView") }),
  createRoute({ getParentRoute: () => root, path: "/settings/profiles", component: lazyRouteComponent(() => import("@/features/settings/ProfilesView"), "ProfilesView") }),
  createRoute({ getParentRoute: () => root, path: "/settings/usage", component: lazyRouteComponent(() => import("@/features/settings/AnalyticsView"), "AnalyticsView") }),
  createRoute({ getParentRoute: () => root, path: "/settings/worktrees", component: lazyRouteComponent(() => import("@/features/settings/WorktreesView"), "WorktreesView") }),
  createRoute({ getParentRoute: () => root, path: "/settings/runners", component: lazyRouteComponent(() => import("@/features/settings/RunnersView"), "RunnersView") }),
  createRoute({ getParentRoute: () => root, path: "/settings/credentials", component: lazyRouteComponent(() => import("@/features/settings/CredentialsView"), "CredentialsView"), validateSearch: (s: Record<string, unknown>): { tool?: string } => (typeof s.tool === "string" && s.tool ? { tool: s.tool } : {}) }),
  createRoute({ getParentRoute: () => root, path: "/settings/channels", component: lazyRouteComponent(() => import("@/features/settings/ChannelsView"), "ChannelsView") }),
  createRoute({ getParentRoute: () => root, path: "/settings/hosts", component: lazyRouteComponent(() => import("@/features/settings/HostsView"), "HostsView") }),
  createRoute({ getParentRoute: () => root, path: "/settings/onboarding", component: lazyRouteComponent(() => import("@/onboarding/OnboardingView"), "OnboardingView") }),
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
