// Phone entry: the shell first; Today, Agents and the other pages load when opened.
import { lazyRouteComponent } from "@tanstack/react-router";
import type { Entry } from "@/app/router";
import { MobileShell } from "./MobileShell";

export const entry: Entry = {
  Root: MobileShell,
  TodayView: lazyRouteComponent(() => import("@/features/today/TodayView"), "TodayView"),
  AgentsView: lazyRouteComponent(() => import("@/features/agents/AgentsView"), "AgentsView"),
};
