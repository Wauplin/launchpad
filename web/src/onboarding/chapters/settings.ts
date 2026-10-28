// Chapter 10: profiles, spending limits, presets, devices and SSH machines.

import type { Chapter } from "../tour/types";
import { clickAnchor } from "./helpers";
import { meta } from "./meta";

const qrShown = () => /demo\.invalid/.test(document.querySelector("main")?.textContent ?? "") || !!document.querySelector("main svg[aria-label*='QR' i], main img[alt*='QR' i]");

export const settings: Chapter = {
  ...meta("settings"),
  learned: ["A profile is one way to run an agent: a harness and what pays for it", "Each subscription login has its own quota meters; pay-as-you-go profiles can have spending limits", "Presets add your usual instructions", "Pair a phone with a QR code; SSH machines run agents elsewhere"],
  steps: [
    {
      id: "settings.profiles", route: "/settings/profiles", anchor: "settings.profiles",
      title: "Profiles",
      body: "One profile per way to pay: a subscription login, an API key or another provider. Subscription profiles get quota meters and protection, one per login, so several accounts work side by side.",
    },
    {
      id: "settings.spend", route: "/settings/profiles", anchor: "settings.profiles",
      title: "Spending limits",
      body: "Pay-as-you-go profiles can have rolling spending limits, here $5 per day, from the cost the agent reports. Reaching one stops new turns; nothing resumes on its own.",
    },
    {
      id: "settings.presets", route: "/settings/presets", anchor: "settings.presets",
      title: "Presets",
      body: "Presets are instructions you add to a launch or a message with one click, like “Write tests”. Launchpad ships none: you write your own.",
    },
    {
      id: "settings.devices", route: "/settings/devices", anchor: "devices.pair",
      title: "Your phone",
      body: "Pair a phone or another browser with a QR code. Every paired browser has full access; the phone layout keeps Now, Agents, Ask, Review and Inbox one tap away.",
      tryIt: { text: "create a pairing link.", done: qrShown, doIt: () => clickAnchor("devices.pair") },
    },
    {
      id: "settings.machines", route: "/settings/hosts", anchor: "settings.machines",
      title: "SSH machines",
      body: "Folders can live on other computers over SSH. Agents run there with the same controls; each machine shows its checks. Credentials are never copied.",
    },
  ],
};
