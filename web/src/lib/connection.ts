/** Only the bundled extension uses a different API origin. Normal web/desktop URLs stay relative. */
export const isExtension = typeof location !== "undefined" && location.protocol === "chrome-extension:";
export function daemonUrl(path: string): string {
  return isExtension ? `${localStorage.getItem("launchpad-daemon-origin") ?? ""}${path}` : path;
}
export function daemonSocket(path: string): string {
  const base = isExtension ? localStorage.getItem("launchpad-daemon-origin")! : location.origin;
  return `${base.replace(/^http/, "ws")}${path}`;
}
/** A download or image link to the daemon. Demo mode (src/demo/) answers it with a data URL instead. */
export function daemonAsset(path: string): string {
  return window.__lpDemo?.asset ? window.__lpDemo.asset(path) : daemonUrl(path);
}
