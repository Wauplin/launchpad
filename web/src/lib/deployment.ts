// Explicit host capability metadata affects presentation only; authentication stays on the server.
import { api } from "./api";
let delegated = false;
let runner = false;
export const delegatedAccess = () => delegated;
/** This window belongs to a runner's own daemon: it shows the runner's status only (`features/runner`). */
export const runnerWindow = () => runner;
export async function loadDeployment() {
  // A runner's daemon answers its window only with its status; everything else is Home's.
  const [config, status] = await Promise.allSettled([api.get<{ delegated?: boolean }>("/api/config"), api.get<{ role?: string }>("/api/status")]);
  /* Offline browsers retain the normal shell until they reconnect. */
  if (config.status === "fulfilled") delegated = config.value.delegated === true;
  if (status.status === "fulfilled") runner = status.value.role === "runner";
}
