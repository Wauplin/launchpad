// Phone entry: the shell and Now first; Agents and Inbox (with the Markdown renderer) load when opened.
import { lazyRouteComponent } from "@tanstack/react-router";
import type { Entry } from "@/app/router";
import { MobileShell } from "./MobileShell";

export const entry: Entry = {
  Root: MobileShell,
  AgentsView: lazyRouteComponent(() => import("@/features/agents/AgentsView"), "AgentsView"),
  InboxView: lazyRouteComponent(() => import("@/features/inbox/InboxView"), "InboxView"),
};
