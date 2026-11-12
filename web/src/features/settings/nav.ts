// The Settings pages, listed by the settings sidebar (app/SettingsSidebar.tsx) and found by its search.
// Kept light: the shell imports it.

import { create } from "zustand";
import type { IconName } from "@/components/Icon";
import { delegatedAccess } from "@/lib/deployment";

/** `keywords`: other words the Settings search finds the page by. */
export type NavEntry = { to: string; label: string; icon: IconName; exact?: boolean; keywords?: string };

/** Settings of the window's workspace. */
export const WORKSPACE_NAV: NavEntry[] = [{ to: "/settings/workspace", label: "Workspace", icon: "folder", keywords: "workspace name rename delete window mcp servers tools bots telegram notifications" }];

/** Launchpad's settings, shared by every workspace. */
export const SETTINGS_NAV: { group: string; items: NavEntry[] }[] = [
  { group: "This device", items: [
    { to: "/settings", label: "Appearance", icon: "sun", exact: true, keywords: "theme dark light zoom layout phone desktop" },
  ] },
  { group: "Agents", items: [
    { to: "/settings/profiles", label: "Profiles", icon: "agents", keywords: "harness claude codex opencode model effort billing subscription pay as you go api key quota" },
    { to: "/settings/voice", label: "Voice", icon: "mic", keywords: "dictation microphone mic speech talk transcribe speak language silence sounds" },
    { to: "/settings/presets", label: "Presets", icon: "preset", keywords: "instructions prompt snippets" },
    { to: "/settings/usage", label: "Usage", icon: "clock", keywords: "analytics activity cost tokens spend month" },
  ] },
  { group: "Machines & access", items: [
    { to: "/settings/runners", label: "Home and runners", icon: "window", keywords: "runner home burst vm jail update connect approve jobs machines computer" },
    { to: "/settings/credentials", label: "Credentials and tools", icon: "sliders", keywords: "vault tokens secrets logins mcp tools proxy launchpad cli access admin" },
    { to: "/settings/connections", label: "Connections", icon: "send", keywords: "webhook outbox hooks messages notifications notify alerts" },
    { to: "/settings/devices", label: "Devices", icon: "window", keywords: "browser pair qr phone sign out tailscale remote access" },
    { to: "/settings/github", label: "GitHub", icon: "pr", keywords: "token account repositories poll rate limit inbox" },
    { to: "/settings/huggingface", label: "Hugging Face", icon: "huggingface", keywords: "traces bucket backup hf token" },
    { to: "/settings/worktrees", label: "Disk cleanup", icon: "archive", keywords: "maintenance disk space worktrees kept trash" },
  ] },
  { group: "Help", items: [{ to: "/settings/onboarding", label: "Onboarding", icon: "info", keywords: "tour demo setup checklist help" }] },
];

/** The Settings search: kept while moving between Settings pages. */
export const useSettingsSearch = create<{ query: string; set: (query: string) => void }>((set) => ({ query: "", set: (query) => set({ query }) }));

/** Every word of `query` starts a word of the page's label or keywords. */
export function matches(e: NavEntry, query: string): boolean {
  const words = `${e.label} ${e.keywords ?? ""}`.toLowerCase().split(/[^a-z0-9]+/);
  return query.toLowerCase().split(/\s+/).filter(Boolean).every((q) => words.some((w) => w.startsWith(q)));
}

/** The pages the sidebar shows for `query`: this workspace's, then Launchpad's groups. Devices is hidden with delegated access. */
export function settingsPages(query: string) {
  const groups = SETTINGS_NAV.map((g) => ({ ...g, items: g.items.filter((e) => (!delegatedAccess() || e.to !== "/settings/devices") && matches(e, query)) })).filter((g) => g.items.length > 0);
  return { workspace: WORKSPACE_NAV.filter((e) => matches(e, query)), groups };
}
