import { createRootRoute, createRoute, createRouter, lazyRouteComponent, redirect, type RouteComponent, type RouterHistory } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { validateCodeSearch } from "@/features/code/store";

/** The shell and the two first pages, loaded with it (app/desktop.ts or mobile/entry.ts). */
export type Entry = { Root: () => ReactNode; TodayView: RouteComponent; AgentsView: RouteComponent };

const SESSION_VIEWS = ["summary", "trace", "changes", "subagents", "processes", "folio"] as const;
type SessionView = (typeof SESSION_VIEWS)[number];

/** Today's address: `filter` (a chip, such as `send`) and `card` (an item ID such as `proposal:<id>`). */
export type TodaySearch = { filter?: string; card?: string };

// Both shells (laptop `Shell`, phone `MobileShell`) share these routes. The laptop entry bundles Today
// and Agents with its shell; every other page is a separate chunk loaded on first visit (or on hover/focus
// of a link, `defaultPreload: "intent"`), so the first paint downloads less.
export function makeRouter({ Root, TodayView, AgentsView }: Entry, history?: RouterHistory) {
const root = createRootRoute({ component: Root });

// Chats (`features/chats`): one layout for the list and its main frame (laptop), so the list stays mounted while
// chats, settings and the landing grid open in the frame.
const chats = createRoute({ getParentRoute: () => root, id: "chats", component: lazyRouteComponent(() => import("@/features/chats/ChatsView"), "ChatsFrame") });
const tabSearch = (s: Record<string, unknown>) => ({ ...(s.new === "channel" ? { new: "channel" as const } : s.new === true || s.new === "true" || s.new === 1 ? { new: true as const } : {}) });
const chatsTree = chats.addChildren([
  // The landing grid (laptop) or the list (phone); `new` holds the New bot and New channel forms, `view=start` the phone's grid.
  createRoute({ getParentRoute: () => chats, path: "/chats", component: lazyRouteComponent(() => import("@/features/chats/ChatsView"), "ChatsView"), validateSearch: (s: Record<string, unknown>): { new?: true | "channel"; view?: "start" } => ({ ...tabSearch(s), ...(s.view === "start" ? { view: "start" as const } : {}) }) }),
  // The former Bots page: its links land on Chats.
  createRoute({ getParentRoute: () => chats, path: "/bots", validateSearch: (s: Record<string, unknown>): { new?: true | "channel" } => tabSearch(s), beforeLoad: ({ search }) => { throw redirect({ to: "/chats", search: search as never, replace: true }); } }),
  // A chat with a bot (`thread`: a thread ID, a routing key such as `channel:<id>`, or `new`), and the bot's settings.
  createRoute({ getParentRoute: () => chats, path: "/bots/$id", component: lazyRouteComponent(() => import("@/features/bots/BotsView"), "BotPage"), validateSearch: (s: Record<string, unknown>): { thread?: string; panel?: string } => ({ ...(typeof s.thread === "string" && s.thread ? { thread: s.thread } : {}), ...(typeof s.panel === "string" && s.panel ? { panel: s.panel } : {}) }) }),
  createRoute({ getParentRoute: () => chats, path: "/bots/$id/settings", component: lazyRouteComponent(() => import("@/features/bots/BotsView"), "BotSettingsPage") }),
  // A bot channel or DM (`features/channels`): its messages, deliveries and your message box; and its settings.
  createRoute({ getParentRoute: () => chats, path: "/channels/$id", component: lazyRouteComponent(() => import("@/features/channels/ChannelPage"), "ChannelPage"), validateSearch: (s: Record<string, unknown>): { panel?: string; thread?: string } => ({ ...(typeof s.panel === "string" && s.panel ? { panel: s.panel } : {}), ...(typeof s.thread === "string" && s.thread ? { thread: s.thread } : {}) }) }),
  createRoute({ getParentRoute: () => chats, path: "/channels/$id/settings", component: lazyRouteComponent(() => import("@/features/channels/ChannelPage"), "ChannelSettingsPage") }),
]);

const routeTree = root.addChildren([
  chatsTree,
  // Today: the home page of both shells.
  createRoute({ getParentRoute: () => root, path: "/", component: TodayView, validateSearch: (s: Record<string, unknown>): TodaySearch => ({ ...(typeof s.filter === "string" && s.filter ? { filter: s.filter } : {}), ...(typeof s.card === "string" && s.card ? { card: s.card } : {}) }) }),
  // Folios: the list, and the full-screen reader (`page`, `rev` and the side `panel` in the address).
  createRoute({ getParentRoute: () => root, path: "/folios", component: lazyRouteComponent(() => import("@/features/folios/FoliosView"), "FoliosView"), validateSearch: (s: Record<string, unknown>): { archived?: true; shared?: true } => (s.archived === true || s.archived === "true" ? { archived: true as const } : s.shared === true || s.shared === "true" ? { shared: true as const } : {}) }),
  createRoute({ getParentRoute: () => root, path: "/folios/$slug", component: lazyRouteComponent(() => import("@/features/folios/FolioReader"), "FolioRoute"), validateSearch: (s: Record<string, unknown>): { page?: string; rev?: number; panel?: "comments" | "versions" | "problems" | "ask" | "share" } => ({ ...(typeof s.page === "string" && s.page ? { page: s.page } : {}), ...(Number.isInteger(Number(s.rev)) && Number(s.rev) > 0 ? { rev: Number(s.rev) } : {}), ...(s.panel === "comments" || s.panel === "versions" || s.panel === "problems" || s.panel === "ask" || s.panel === "share" ? { panel: s.panel } : {}) }) }),
  createRoute({ getParentRoute: () => root, path: "/topics", component: lazyRouteComponent(() => import("@/features/topics/TopicsView"), "TopicsView") }),
  createRoute({ getParentRoute: () => root, path: "/topics/$id", component: lazyRouteComponent(() => import("@/features/topics/TopicsView"), "TopicDetail") }),
  // Phone pages.
  createRoute({ getParentRoute: () => root, path: "/terminals", component: lazyRouteComponent(() => import("@/mobile/TerminalsView"), "TerminalsView") }),
  createRoute({ getParentRoute: () => root, path: "/more", component: lazyRouteComponent(() => import("@/mobile/MoreView"), "MoreView") }),
  createRoute({ getParentRoute: () => root, path: "/schedules", component: lazyRouteComponent(() => import("@/features/schedules/SchedulesView"), "SchedulesView"), validateSearch: (s: Record<string, unknown>): { tab?: "routines" | "launches" | "oracles"; routine?: string; launch?: string; oracle?: string; view?: string } => ({ ...(s.tab === "routines" || s.tab === "launches" || s.tab === "oracles" ? { tab: s.tab } : {}), ...(typeof s.routine === "string" && s.routine ? { routine: s.routine } : {}), ...(typeof s.launch === "string" && s.launch ? { launch: s.launch } : {}), ...(typeof s.oracle === "string" && s.oracle ? { oracle: s.oracle } : {}), ...(typeof s.view === "string" && s.view ? { view: s.view } : {}) }) }),
  createRoute({ getParentRoute: () => root, path: "/predrafts/$id", component: lazyRouteComponent(() => import("@/features/predrafts/PredraftView"), "PredraftView") }),
  createRoute({
    getParentRoute: () => root,
    path: "/agents",
    component: AgentsView,
    // `view` opens one view of the session (`changes` from Review changes links; the phone keeps the current one).
    validateSearch: (s: Record<string, unknown>): { session?: string; view?: SessionView; topic?: string } => ({ ...(typeof s.topic === "string" ? { topic: s.topic } : {}), ...(typeof s.session === "string" ? { session: s.session } : {}), ...(typeof s.view === "string" && SESSION_VIEWS.includes(s.view as SessionView) ? { view: s.view as SessionView } : {}) }),
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
  createRoute({ getParentRoute: () => root, path: "/settings/runners", component: lazyRouteComponent(() => import("@/features/settings/RunnersView"), "RunnersView"), validateSearch: (s: Record<string, unknown>): { route?: string } => (typeof s.route === "string" && s.route ? { route: s.route } : {}) }),
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
