// What this client can do. The desktop shell (Tauri) marks itself with `window.__launchpadDesktop`; everything
// else is a browser tab, possibly on another device than the daemon.

import { api } from "./api";
import { toastError } from "@/components/Toast";

declare global {
  interface Window { __launchpadDesktop?: boolean }
}

/** True inside the Launchpad desktop window, which runs on the daemon's machine. */
export const isDesktopShell = typeof window !== "undefined" && window.__launchpadDesktop === true;

/** Opens an external link on the device the user is looking at: the daemon's browser for the desktop shell, a new tab otherwise. */
export function openExternal(url: string) {
  if (isDesktopShell) {
    void api.post("/api/open", { url }).catch(toastError);
    return;
  }
  window.open(url, "_blank", "noopener,noreferrer");
}

/** Props for an `<a>` to an external page; the click goes through `openExternal` in the desktop shell. */
export function externalLinkProps(url: string) {
  return {
    href: url,
    target: "_blank",
    rel: "noopener noreferrer",
    onClick: (e: { preventDefault: () => void }) => {
      if (!isDesktopShell) return;
      e.preventDefault();
      openExternal(url);
    },
  };
}
