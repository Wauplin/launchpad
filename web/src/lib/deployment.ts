// Explicit host capability metadata affects presentation only; authentication stays on the server.
import { api } from "./api";
let delegated = false;
let setupUrl: string | null = null;
export const externalSetup = () => setupUrl;
export const delegatedAccess = () => delegated;
export async function loadDeployment() {
  try {
    const value = await api.get<{ delegated?: boolean; external_setup?: string | null }>("/api/config");
    delegated = value.delegated === true;
    setupUrl = value.external_setup?.startsWith("/") && !value.external_setup.startsWith("//") ? value.external_setup : null;
  } catch { /* Offline browsers retain the normal shell until they reconnect. */ }
}
