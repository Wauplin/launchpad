/** API URLs stay relative: the app is served by the daemon it talks to. */
export function daemonUrl(path: string): string {
  return path;
}
export function daemonSocket(path: string): string {
  return `${location.origin.replace(/^http/, "ws")}${path}`;
}
/** A download or image link to the daemon. Demo mode (src/demo/) answers it with a data URL instead. */
export function daemonAsset(path: string): string {
  return window.__lpDemo?.asset ? window.__lpDemo.asset(path) : daemonUrl(path);
}
