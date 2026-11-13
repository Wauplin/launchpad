// Small helpers of the side panel: links into Launchpad and URL parts.

import { daemonUrl } from "@/lib/connection";

export const isWebPage = (url: string | undefined) => !!url && /^https?:/.test(url);
export const siteOf = (url: string) => { try { return new URL(url).origin; } catch { return ""; } };
export const hostOf = (url: string) => { try { return new URL(url).hostname.replace(/^www\./, ""); } catch { return url; } };
export const openInLaunchpad = (path: string) => void chrome.tabs.create({ url: daemonUrl(path) });
