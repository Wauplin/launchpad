// Small helpers of the side panel: links into Launchpad and URL parts.

import type { HomeInfo } from "./shared/messages";

export const isWebPage = (url: string | undefined) => !!url && /^https?:/.test(url);
export const siteOf = (url: string) => { try { return new URL(url).origin; } catch { return ""; } };
export const hostOf = (url: string) => { try { return new URL(url).hostname.replace(/^www\./, ""); } catch { return url; } };
/** Opens a page of the connected Home in a new tab. */
export const openInLaunchpad = (home: HomeInfo, path = "/") => void chrome.tabs.create({ url: home.origin + path });
