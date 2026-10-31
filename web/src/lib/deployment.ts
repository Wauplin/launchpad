// Explicit host capability metadata affects presentation only; authentication stays on the server.
import { api } from "./api";
let delegated = false;
let managedTrace = false;
export const managedTraceDestination = () => managedTrace;
let setupUrl: string | null = null;
export const externalSetup = () => setupUrl;
export const delegatedAccess = () => delegated;
export async function loadDeployment() {
  try {
    const value = await api.get<{ delegated?: boolean; external_setup?: string | null; managed_trace_destination?: boolean }>("/api/config");
    delegated = value.delegated === true;
    managedTrace = value.managed_trace_destination === true;
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
