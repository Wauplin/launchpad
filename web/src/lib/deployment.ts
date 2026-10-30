// Explicit host capability metadata affects presentation only; authentication stays on the server.
import { api } from "./api";
let delegated = false;
export const delegatedAccess = () => delegated;
export async function loadDeployment() {
  try {
    const value = await api.get<{ delegated?: boolean }>("/api/config");
    delegated = value.delegated === true;
  } catch { /* Offline browsers retain the normal shell until they reconnect. */ }
}
