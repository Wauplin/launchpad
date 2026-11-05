// Chapter 1: demo mode, the five places, workspaces and quota meters.

import type { Chapter } from "../tour/types";
import { useWorkspace } from "@/stores/workspace";
import { WS } from "@/demo/world/ids";
import { clickButton } from "./helpers";
import { meta } from "./meta";

let sawPersonal = false;

export const layout: Chapter = {
  ...meta("layout"),
  learned: ["Demo mode and how to leave it", "Where Ask, Inbox, Agents, Review and Folders are", "Each window shows one workspace; the rail switches and counts what waits in the others", "Quota meters show what your subscriptions have left"],
  setup: () => { sawPersonal = false; useWorkspace.getState().switchTo(WS.acme); },
  steps: [
    {
      id: "layout.demo", route: "/agents", anchor: "demo.chip",
      title: "This is a demo",
      body: "Everything here is fake: agents replay prerecorded runs and nothing reaches GitHub, your folders or your settings. The purple line and this chip show it; Exit demo leaves at any time.",
    },
    {
      id: "layout.places", route: "/agents", anchor: "sidebar.nav",
      title: "The places you use every day",
      body: "Ask Launchpad answers questions about your agents. Inbox holds your GitHub notifications. Agents lists every run. Review collects what waits for publication. Counts show what needs you.",
    },
    {
      id: "layout.workspaces", route: "/agents", anchor: "rail.workspaces",
      title: "Workspaces",
      body: "Each window shows one workspace: its folders, agents, bots, Inbox and notes. The rail lists your workspaces. A badge counts what waits in another workspace, never its content. Click a square to switch; open a second window to see two side by side.",
      tryIt: {
        text: "switch to the Personal workspace, then back to Acme.",
        done: () => { const ws = useWorkspace.getState().current; if (ws === WS.personal) sawPersonal = true; return sawPersonal && ws !== WS.personal; },
        doIt: async () => { await clickButton("Personal", "rail.workspaces"); await new Promise((r) => setTimeout(r, 700)); await clickButton("Acme", "rail.workspaces"); },
      },
    },
    {
      id: "layout.quota", route: "/agents", anchor: "statusbar.quota",
      title: "Quota meters",
      body: "Each subscription login shows its 5-hour and weekly usage. Launchpad pauses protected agents before paid usage starts. Click a meter pair for details and reset times.",
    },
  ],
};
