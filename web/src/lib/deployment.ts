// Explicit host capability metadata affects presentation only; authentication stays on the server.
import { api } from "./api";
let delegated = false;
let setupUrl: string | null = null;
export const externalSetup = () => setupUrl;
export const delegatedAccess = () => delegated;
export async function loadDeployment() {
  try {
    const value = await api.get<{ delegated?: boolean; external_setup?: string | null}>("/api/config");
    delegated = value.delegated === true;
    setupUrl = value.external_setup?.startsWith("/") && !value.external_setup.startsWith("//") ? value.external_setup : null;
  } catch { /* Offline browsers retain the normal shell until they reconnect. */ }
}

export function connectionSetup(profile: string): string | null {
  if (!setupUrl) return null;
  const url = new URL(setupUrl, location.origin);
  url.searchParams.set("profile", profile);
  url.searchParams.set("route", location.pathname + location.search + location.hash);
  return url.pathname + url.search;
}

/** Opens the host's settings panel. Inside the Space shell (our parent frame) this asks it to open its drawer;
 * a `target="_top"` link would try to navigate the Hub page that embeds the Space, which browsers block. */
export function openSpaceSettings(profile?: string) {
  if (window.parent !== window) {
    window.parent.postMessage({ type: "launchpad-space-settings", profile: profile ?? null }, location.origin);
    return;
  }
  const url = profile ? connectionSetup(profile) : setupUrl;
  if (url) location.assign(url);
}
