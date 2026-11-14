// Chapter 10: profiles, spending limits, presets, devices, and Home's GitHub login.

import type { Chapter } from "../tour/types";
import { clickAnchor } from "./helpers";
import { meta } from "./meta";

const qrShown = () => /demo\.invalid/.test(document.querySelector("main")?.textContent ?? "") || !!document.querySelector("main svg[aria-label*='QR' i], main img[alt*='QR' i]");

export const settings: Chapter = {
  ...meta("settings"),
  learned: ["A profile is one way to run an agent: a harness and what pays for it", "Each subscription login has its own quota meters; pay-as-you-go profiles can have spending limits", "Presets add your usual instructions", "Pair a phone with a QR code", "Settings has two parts: Workspace, and Launchpad, shared by every workspace", "Each workspace can have its own Telegram bot and chat", "Settings → GitHub shows Home's GitHub login and rate limits"],
  steps: [
    {
      id: "settings.trace-sync", route: "/settings/huggingface", anchor: "settings.trace-sync",
      title: "Traces in your bucket",
      body: "Home copies the original conversation of every agent to its backup bucket, one file per trace, under <workspace>/traces/<folder>/. Open them with the bucket's trace viewer on Hugging Face.",
    },
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
      body: "Presets are instructions you add to a launch or a message with one click, like “Write tests”. Choose availability in Ask Launchpad, agent composer / steering, or both. Launchpad ships none: you write your own.",
    },
    {
      id: "settings.devices", route: "/settings/devices", anchor: "devices.pair",
      title: "Your phone",
      body: "Pair a phone or another browser with a QR code. Every paired browser has full access; the phone layout keeps Today, Chats and Agents one tap away, and every other place in More.",
      tryIt: { text: "create a pairing link.", done: qrShown, doIt: () => clickAnchor("devices.pair") },
    },
    {
      id: "settings.connections", route: "/settings/connections", anchor: "settings.connections",
      title: "Connections",
      body: "Add Slack, Discord or webhook connections for agents' messages to other people: those wait in Today, under Send. Incoming hooks let CI and scripts message a bot.",
    },
    {
      id: "settings.telegram", route: "/settings/workspace", anchor: "settings.telegram",
      title: "Telegram, per workspace",
      body: "Each workspace can have its own Telegram bot and chat. Plain text goes to the workspace's Ask bot, /<bot> to its other bots, and only its own notifications arrive there, so two workspaces never mix.",
    },
    {
      id: "settings.mcp", route: "/settings/credentials", anchor: "settings.mcp",
      title: "MCP servers",
      body: "Add a remote MCP server once, with its token: agents on every machine reach it through Home, and the token never leaves Home. Switch off each tool agents must not use; a launch can leave a server out.",
    },
    {
      id: "settings.github", route: "/settings/github", anchor: "settings.github-limits",
      title: "GitHub",
      body: "Home's GitHub login clones repositories, publishes Local PRs and posts suggestions; it reads GitHub only when one of these asks, never in the background. Rate limits show what is left as of Home's last call. This page is under Launchpad, shared by every workspace; Workspace, at the top, holds its name, bots, Telegram, tools and MCP servers.",
    },
  ],
};
