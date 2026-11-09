// What this client can do: Launchpad runs in a browser tab or as an installed web app, possibly on another
// device than the daemon.

/** Opens an external link in a new tab on the device the user is looking at. */
export function openExternal(url: string) {
  window.open(url, "_blank", "noopener,noreferrer");
}

/** Props for an `<a>` to an external page. */
export function externalLinkProps(url: string) {
  return { href: url, target: "_blank", rel: "noopener noreferrer" };
}

/** The installed app's icon badge follows the Today count of every workspace where the platform supports it. */
export function setAppBadge(count: number) {
  const nav = navigator as Navigator & { setAppBadge?: (n?: number) => Promise<void>; clearAppBadge?: () => Promise<void> };
  if (!nav.setAppBadge) return;
  void (count > 0 ? nav.setAppBadge(count) : nav.clearAppBadge?.())?.catch(() => {});
}
