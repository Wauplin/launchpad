import { createRootRoute, createRoute, createRouter, lazyRouteComponent, type RouteComponent, type RouterHistory } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { validateCodeSearch } from "@/features/code/store";

/** The shell and the two first pages, loaded with it (app/desktop.ts or mobile/entry.ts). */
export type Entry = { Root: () => ReactNode; TodayView: RouteComponent; AgentsView: RouteComponent };

const SESSION_VIEWS = ["summary", "trace", "changes", "subagents", "processes"] as const;
type SessionView = (typeof SESSION_VIEWS)[number];

/** Today's address: `filter` (a chip, such as `send`) and `card` (an item ID such as `proposal:<id>`). */
export type TodaySearch = { filter?: string; card?: string };

// Both shells (laptop `Shell`, phone `MobileShell`) share these routes. The laptop entry bundles Today
// and Agents with its shell; every other page is a separate chunk loaded on first visit (or on hover/focus
// of a link, `defaultPreload: "intent"`), so the first paint downloads less.
export function makeRouter({ Root, TodayView, AgentsView }: Entry, history?: RouterHistory) {
const root = createRootRoute({ component: Root });

const routeTree = root.addChildren([
  // Today: the home page of both shells.
  createRoute({ getParentRoute: () => root, path: "/", component: TodayView, validateSearch: (s: Record<string, unknown>): TodaySearch => ({ ...(typeof s.filter === "string" && s.filter ? { filter: s.filter } : {}), ...(typeof s.card === "string" && s.card ? { card: s.card } : {}) }) }),
  // Phone pages.
  createRoute({ getParentRoute: () => root, path: "/terminals", component: lazyRouteComponent(() => import("@/mobile/TerminalsView"), "TerminalsView") }),
  createRoute({ getParentRoute: () => root, path: "/more", component: lazyRouteComponent(() => import("@/mobile/MoreView"), "MoreView") }),
  // `thread`: the phone shows one Inbox thread (a history entry, so back returns to the list).
  createRoute({ getParentRoute: () => root, path: "/inbox", component: lazyRouteComponent(() => import("@/features/inbox/InboxView"), "InboxView"), validateSearch: (s: Record<string, unknown>): { thread?: string } => (typeof s.thread === "string" && s.thread ? { thread: s.thread } : {}) }),
  // The phone's Chats tab: every bot of the workspace, Ask Launchpad first.
  createRoute({ getParentRoute: () => root, path: "/chats", component: lazyRouteComponent(() => import("@/mobile/ChatsView"), "ChatsView") }),
  createRoute({ getParentRoute: () => root, path: "/bots", component: lazyRouteComponent(() => import("@/features/bots/BotsView"), "BotsView"), validateSearch: (s: Record<string, unknown>): { new?: boolean } => (s.new === true || s.new === "true" || s.new === 1 ? { new: true } : {}) }),
  createRoute({ getParentRoute: () => root, path: "/bots/$id", component: lazyRouteComponent(() => import("@/features/bots/BotsView"), "BotPage"), validateSearch: (s: Record<string, unknown>): { panel?: string } => (typeof s.panel === "string" && s.panel ? { panel: s.panel } : {}) }),
  createRoute({ getParentRoute: () => root, path: "/schedules", component: lazyRouteComponent(() => import("@/features/schedules/SchedulesView"), "SchedulesView"), validateSearch: (s: Record<string, unknown>): { tab?: "routines" | "launches"; routine?: string; launch?: string } => ({ ...(s.tab === "routines" || s.tab === "launches" ? { tab: s.tab } : {}), ...(typeof s.routine === "string" && s.routine ? { routine: s.routine } : {}), ...(typeof s.launch === "string" && s.launch ? { launch: s.launch } : {}) }) }),
  createRoute({ getParentRoute: () => root, path: "/predrafts/$id", component: lazyRouteComponent(() => import("@/features/predrafts/PredraftView"), "PredraftView") }),
  createRoute({
    getParentRoute: () => root,
    path: "/github/$owner/$repo/$number",
    component: lazyRouteComponent(() => import("@/features/github/ItemView"), "ItemView"),
    // `draft`: the agent suggestion to expand and scroll to. `tab`: the PR view (Conversation without it). `file`:
    // the file a PR's Changes shows (the phone's one-file view).
    validateSearch: (s: Record<string, unknown>): { draft?: string; tab?: "files" | "threads"; file?: string } => ({
      ...(typeof s.draft === "string" || typeof s.draft === "number" ? { draft: String(s.draft) } : {}),
      ...(s.tab === "files" || s.tab === "threads" ? { tab: s.tab } : {}),
      ...(typeof s.file === "string" && s.file ? { file: s.file } : {}),
    }),
  }),
  createRoute({
    getParentRoute: () => root,
    path: "/agents",
    component: AgentsView,
    // `view` opens one view of the session (`changes` from Review changes links; the phone keeps the current one).
    validateSearch: (s: Record<string, unknown>): { session?: string; view?: SessionView } => ({ ...(typeof s.session === "string" ? { session: s.session } : {}), ...(typeof s.view === "string" && SESSION_VIEWS.includes(s.view as SessionView) ? { view: s.view as SessionView } : {}) }),
  }),
  createRoute({ getParentRoute: () => root, path: "/projects", component: lazyRouteComponent(() => import("@/features/projects/ProjectsView"), "ProjectsView") }),
  createRoute({ getParentRoute: () => root, path: "/projects/$id", component: lazyRouteComponent(() => import("@/features/projects/ProjectPage"), "ProjectPage") }),
  createRoute({ getParentRoute: () => root, path: "/projects/$id/code", component: lazyRouteComponent(() => import("@/features/code/CodeView"), "CodeView"), validateSearch: validateCodeSearch }),
  createRoute({ getParentRoute: () => root, path: "/knowledge", component: lazyRouteComponent(() => import("@/features/knowledge/KnowledgeView"), "KnowledgeView"), validateSearch: (s: Record<string, unknown>): { note?: string } => (typeof s.note === "string" && s.note ? { note: s.note } : {}) }),
  createRoute({ getParentRoute: () => root, path: "/setup", component: lazyRouteComponent(() => import("@/features/setup/SetupView"), "SetupView") }),
  createRoute({ getParentRoute: () => root, path: "/settings", component: lazyRouteComponent(() => import("@/features/settings/SettingsView"), "SettingsView") }),
  createRoute({ getParentRoute: () => root, path: "/settings/workspace", component: lazyRouteComponent(() => import("@/features/settings/WorkspaceSettings"), "WorkspaceSettings") }),
  createRoute({ getParentRoute: () => root, path: "/settings/presets", component: lazyRouteComponent(() => import("@/features/settings/SettingsView"), "PresetsView") }),
  createRoute({ getParentRoute: () => root, path: "/settings/huggingface", component: lazyRouteComponent(() => import("@/features/settings/HuggingFaceView"), "HuggingFaceView") }),
  createRoute({ getParentRoute: () => root, path: "/settings/github", component: lazyRouteComponent(() => import("@/features/settings/GitHubView"), "GitHubView") }),
  createRoute({ getParentRoute: () => root, path: "/settings/profiles", component: lazyRouteComponent(() => import("@/features/settings/ProfilesView"), "ProfilesView") }),
  createRoute({ getParentRoute: () => root, path: "/settings/usage", component: lazyRouteComponent(() => import("@/features/settings/AnalyticsView"), "AnalyticsView") }),
  createRoute({ getParentRoute: () => root, path: "/settings/worktrees", component: lazyRouteComponent(() => import("@/features/settings/WorktreesView"), "WorktreesView") }),
  createRoute({ getParentRoute: () => root, path: "/settings/runners", component: lazyRouteComponent(() => import("@/features/settings/RunnersView"), "RunnersView") }),
  createRoute({ getParentRoute: () => root, path: "/settings/credentials", component: lazyRouteComponent(() => import("@/features/settings/CredentialsView"), "CredentialsView"), validateSearch: (s: Record<string, unknown>): { tool?: string } => (typeof s.tool === "string" && s.tool ? { tool: s.tool } : {}) }),
  createRoute({ getParentRoute: () => root, path: "/settings/connections", component: lazyRouteComponent(() => import("@/features/settings/ConnectionsView"), "ConnectionsView") }),
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
