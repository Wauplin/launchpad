// Chapter 11: Home and runners. The demo switches to the Home role: Home keeps every record and credential and runs
// no agent; runners (the laptop, Burst jobs) run them, with tokens added by Home.

import type { Chapter } from "../tour/types";
import { publish } from "@/demo/daemon";
import { world } from "@/demo/world";
import { meta } from "./meta";

export const home: Chapter = {
  ...meta("home"),
  learned: ["Home keeps every record and credential; runners run the agents", "Burst runs a session on an HF Job when no runner is online, with no token inside", "Logins and tokens are saved once on Home; agents use them without holding them"],
  setup: () => {
    world.status = { ...world.status, role: "home" };
    publish("status", "changed", world.status);
  },
  steps: [
    {
      id: "home.runners", route: "/settings/runners", anchor: "settings.runners",
      title: "Home and runners",
      body: "Home, a private Space, keeps every record and runs no agent. Runners do: your laptop, paired once with the command on this page, takes launches from the phone, schedules and bots while it is online.",
    },
    {
      id: "home.burst", route: "/settings/runners", anchor: "settings.burst",
      title: "Burst when the laptop sleeps",
      body: "When no runner is online, Home runs the session on Burst: a small HF Job that holds no token and stops after the turn. Your next message starts a new one. A monthly cap bounds the cost.",
    },
    {
      id: "home.vault", route: "/settings/credentials", anchor: "settings.vault",
      title: "Credentials, saved once",
      body: "Harness logins and tokens live on Home only. Agents run in a jail and never see them: the runner's proxy adds them to requests for each tool's hosts, and every use is logged in the session.",
    },
  ],
};
