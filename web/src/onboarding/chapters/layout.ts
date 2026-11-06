// Chapter 1: demo mode, Today and the other places, workspaces and quota meters.

import type { Chapter } from "../tour/types";
import { useWorkspace } from "@/stores/workspace";
import { WS } from "@/demo/world/ids";
import { clickButton } from "./helpers";
import { meta } from "./meta";

let sawPersonal = false;

export const layout: Chapter = {
  ...meta("layout"),
  learned: ["Demo mode and how to leave it", "Today lists every decision; Agents, Inbox and Folders are one click away", "Each window shows one workspace; the rail switches and counts what waits in the others", "Quota meters show what your subscriptions have left"],
  setup: () => { sawPersonal = false; useWorkspace.getState().switchTo(WS.acme); },
  steps: [
    {
      id: "layout.demo", route: "/", anchor: "demo.chip",
      title: "This is a demo",
      body: "Everything here is fake: agents replay prerecorded runs and nothing reaches GitHub, your folders or your settings. The purple line and this chip show it; Exit demo leaves at any time.",
    },
    {
      id: "layout.today", route: "/", anchor: "today.filters",
      title: "Today",
      body: "Today is your home: every decision of the workspace on one page, in the order to make it. One compact row per decision, grouped by kind: answer questions, merge finished work, start proposals, send drafts and post reviews from the rows. The icon says what a row is; its title opens the details. The chips filter by kind; Done lists what you handled in the last 30 days.",
    },
    {
      id: "layout.places", route: "/", anchor: "sidebar.nav",
      title: "The places you use every day",
      body: "Today carries the one count that needs you. Agents shows what runs and what ran. Inbox holds your GitHub notifications. Bots sit above, Schedules, Knowledge and Folders below; Ask Launchpad is the first bot.",
    },
    {
      id: "layout.workspaces", route: "/", anchor: "rail.workspaces",
      title: "Workspaces",
      body: "Each window shows one workspace: its folders, agents, bots, Inbox and notes. The rail lists your workspaces. A badge shows the Today count of another workspace, never its content. Click a square to switch; open a second window to see two side by side.",
      tryIt: {
        text: "switch to the Personal workspace, then back to Acme.",
        done: () => { const ws = useWorkspace.getState().current; if (ws === WS.personal) sawPersonal = true; return sawPersonal && ws !== WS.personal; },
        doIt: async () => { await clickButton("Personal", "rail.workspaces"); await new Promise((r) => setTimeout(r, 700)); await clickButton("Acme", "rail.workspaces"); },
      },
    },
    {
      id: "layout.quota", route: "/", anchor: "statusbar.quota",
      title: "Quota meters",
      body: "Each subscription login shows its 5-hour and weekly usage. Launchpad pauses protected agents before paid usage starts. Click a meter pair for details and reset times.",
    },
  ],
};
