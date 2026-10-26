// Daemon calls, Chrome tab helpers and small preferences of the side panel.

import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { daemonUrl } from "@/lib/connection";
import type { Session } from "@/gen/Session";

export type Page = { tab: number; document: string; url: string; title: string };
export type Grant = { id: string; session: string | null; page: Page; device: string; touched: number; work: boolean; paused: boolean; uncertain: boolean };
export type Review = { id: string; session: string; details: { tool: string; args: Record<string, unknown>; target: { url?: string; targets?: { ref: string; label: string; value: string }[] }; page: Page } };
/** A tab chosen in the box. `activeTab`: shared from the context menu, which already grants access to it. */
export type TabChip = { tab: number; title: string; url: string; favIconUrl?: string; activeTab?: boolean };

export const GRANT_TTL = 1_800_000;
export const live = (g: Grant) => g.touched > Date.now() - GRANT_TTL;
export const isBrowserTask = (s: Session) => s.launch_spec.target.kind === "browser";
export const isWebPage = (url: string | undefined) => !!url && /^https?:/.test(url);
export const siteOf = (url: string) => { try { return new URL(url).origin; } catch { return ""; } };
export const hostOf = (url: string) => { try { return new URL(url).hostname.replace(/^www\./, ""); } catch { return url; } };
export const openInLaunchpad = (path: string) => void chrome.tabs.create({ url: daemonUrl(path) });

export const useGrants = () => useQuery({ queryKey: ["browser-grants"], queryFn: () => api.get<Grant[]>("/api/browser/grants"), refetchInterval: 5000 });
export const useReviews = () => useQuery({ queryKey: ["browser-reviews"], queryFn: () => api.get<Review[]>("/api/browser/reviews"), refetchInterval: 1000 });
export const useSites = () => useQuery({ queryKey: ["browser-sites"], queryFn: () => api.get<string[]>("/api/browser/sites"), staleTime: 60_000 });

type Prefs = { profile: string | null; act: boolean };
const PREFS = "launchpad-extension-prefs";
export function prefs(): Prefs {
  try { return { profile: null, act: false, ...JSON.parse(localStorage.getItem(PREFS) ?? "{}") }; } catch { return { profile: null, act: false }; }
}
export function savePrefs(patch: Partial<Prefs>) { localStorage.setItem(PREFS, JSON.stringify({ ...prefs(), ...patch })); }

/**
 * Asks Chrome for access to the chips' sites. Must run first in the click handler: Chrome only shows
 * the permission prompt during the user gesture.
 */
export function requestSites(chips: TabChip[]): Promise<boolean> {
  const origins = [...new Set(chips.filter((c) => !c.activeTab).map((c) => `${siteOf(c.url)}/*`))];
  return origins.length ? chrome.permissions.request({ origins }) : Promise.resolve(true);
}

/** Reads each chosen tab and creates its grant. Nothing is shared with an agent until the grant is claimed. */
export async function grantTabs(chips: TabChip[]): Promise<Grant[]> {
  const grants: Grant[] = [];
  for (const chip of chips) {
    if (!isWebPage(chip.url)) throw new Error(`${chip.title}: Chrome pages and the Chrome Web Store cannot be shared.`);
    const captured = await chrome.runtime.sendMessage({ type: "capture", tab: chip.tab });
    if (!captured) throw new Error("The extension worker is unavailable. Reload the extension and try again.");
    if (captured.error) throw new Error(`${chip.title}: ${captured.error}`);
    grants.push(await api.post<Grant>("/api/browser/grants", captured));
  }
  return grants;
}

/** A "New tab" handle: the agent opens its own tabs through it (no page shared). */
export const newTabGrant = () => api.post<Grant>("/api/browser/grants/new-tab");
export const isHandle = (g: Grant) => g.page.tab < 0;

export async function tabChip(tabId: number): Promise<TabChip | null> {
  try {
    const t = await chrome.tabs.get(tabId);
    return t.id == null ? null : { tab: t.id, title: t.title || hostOf(t.url ?? ""), url: t.url ?? "", favIconUrl: t.favIconUrl };
  } catch { return null; }
}

/** Short, plain description of a tool call for the step list. */
export function stepLabel(name: string, input: unknown, summary: string, grants: Grant[]): string {
  const tool = /browser_([a-z_]+)/.exec(name)?.[1];
  const raw = (input && typeof input === "object" ? input : {}) as Record<string, unknown>;
  // Codex nests MCP arguments; Claude and OpenCode pass them directly.
  const args = (raw.arguments && typeof raw.arguments === "object" ? raw.arguments : raw) as Record<string, unknown>;
  const tab = grants.find((g) => g.id === args.tab)?.page.title;
  const on = tab ? ` “${tab}”` : "";
  const url = typeof args.url === "string" ? hostOf(args.url) : "";
  switch (tool) {
    case "tabs": return "Checked the shared tabs";
    case "read": return `Read${on || " a page"}`;
    case "outline": return `Looked at${on || " the page"}`;
    case "click": return `Clicked on${on || " the page"}`;
    case "type": return `Typed in${on || " a field"}`;
    case "select": return `Picked an option${tab ? ` in “${tab}”` : ""}`;
    case "press": return `Pressed ${String(args.key ?? "a key")}`;
    case "scroll": return "Scrolled";
    case "navigate": return `Went to ${url || "another page"}`;
    case "open_tab": return `Opened ${url || "a new tab"}`;
    case "wait": return "Waited for the page";
    case "propose_form": return "Proposed form values";
    case "handover": return "Asked you to take over";
    case "trace": return "Read an earlier task";
  }
  return summary || name;
}

/** Remembers (on) or forgets (off) the chips' sites as always allowed for Act. */
export async function rememberSites(always: { sites: string[]; on: boolean } | undefined, known: string[]) {
  if (!always) return;
  for (const site of always.sites) {
    if (always.on && !known.includes(site)) await api.post("/api/browser/sites", { url: site });
    if (!always.on && known.includes(site)) await api.post("/api/browser/sites/forget", { origin: site });
  }
}
