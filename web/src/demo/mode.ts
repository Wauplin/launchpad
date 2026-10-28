// Demo mode flag, set before the app loads by the inline script of index.html (`?demo=<chapter>` enters it,
// the tab's `window.name` keeps it across reloads). Keep this module tiny: the shells import it.

declare global {
  interface Window {
    __lpDemo?: { chapter: string; realFetch: typeof fetch; ready: (daemon: { fetch: typeof fetch }) => void; asset?: (path: string) => string };
  }
}

export const isDemo = typeof window !== "undefined" && !!window.__lpDemo;

/** The tour chapter the demo starts with ("" explores freely). */
export const demoChapter = () => window.__lpDemo?.chapter ?? "";

/** Opens the demo in this tab (a full page load), starting `chapter` ("" explores freely). */
export function enterDemo(chapter = "", path = "/agents") {
  window.name = `lp-demo:${chapter}`;
  location.assign(path);
}

/** Leaves the demo: a full page load without the flag, so every demo object is gone. */
export function exitDemo(path = "/settings/onboarding") {
  window.name = "";
  location.assign(path);
}

/** Reloads the demo at `chapter`, from its starting state. */
export function restartDemo(chapter: string, path: string) {
  window.name = `lp-demo:${chapter}`;
  location.assign(path);
}
