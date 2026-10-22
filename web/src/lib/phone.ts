import { isExtension } from "./connection.ts";

// Phone layout: a separate shell (bottom tabs, one pane at a time, touch-sized controls) for narrow screens.
// Chosen once per page load: automatic below 720 CSS px, or forced per device in local storage. Crossing the
// breakpoint reloads the page, so the right entry (and only its code) loads.

export type LayoutChoice = "auto" | "phone" | "desktop";
const key = "launchpad-layout";
const query = "(max-width: 719px)";

export function layoutChoice(): LayoutChoice {
  try {
    const v = localStorage.getItem(key);
    return v === "phone" || v === "desktop" ? v : "auto";
  } catch { return "auto"; }
}

function decide(): boolean {
  if (isExtension) return true;
  const choice = layoutChoice();
  if (choice !== "auto") return choice === "phone";
  return typeof window !== "undefined" && !window.__launchpadDesktop && window.matchMedia(query).matches;
}

/** True when this page runs the phone shell. Fixed for the life of the page. */
export const isPhone = decide();

/** Same value as `isPhone`, as a hook for components shared by both shells. */
export const usePhone = () => isPhone;

export function setLayoutChoice(choice: LayoutChoice) {
  try {
    if (choice === "auto") localStorage.removeItem(key);
    else localStorage.setItem(key, choice);
  } catch { /* Keep the current layout. */ }
  location.reload();
}

/** Reloads into the other shell when an automatic layout crosses the breakpoint (rotating a tablet, resizing a window). */
export function watchLayout() {
  if (layoutChoice() !== "auto" || window.__launchpadDesktop) return;
  const mq = window.matchMedia(query);
  let timer: ReturnType<typeof setTimeout> | undefined;
  mq.addEventListener("change", () => {
    clearTimeout(timer);
    timer = setTimeout(() => { if (mq.matches !== isPhone) location.reload(); }, 600);
  });
}
