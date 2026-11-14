
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
  const choice = layoutChoice();
  if (choice !== "auto") return choice === "phone";
  return typeof window !== "undefined" && typeof window.matchMedia === "function" && window.matchMedia(query).matches;
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
  if (layoutChoice() !== "auto") return;
  const mq = window.matchMedia(query);
  let timer: ReturnType<typeof setTimeout> | undefined;
  mq.addEventListener("change", () => {
    clearTimeout(timer);
    timer = setTimeout(() => { if (mq.matches !== isPhone) location.reload(); }, 600);
  });
}

/** Hugging Face's Space page (huggingface.co/spaces/…) frames Home's `*.hf.space` address and draws its own floating
 * button over the bottom-right corner of the frame. The phone shell then keeps that corner free (`lp-hf-corner` in
 * styles.css). `?hf-corner=1` forces it, for screenshots. */
export const hfCorner = (() => {
  if (typeof window === "undefined") return false;
  if (new URLSearchParams(location.search).get("hf-corner") === "1") return true;
  let framed = true;
  try { framed = window.self !== window.top; } catch { /* A cross-origin parent: framed. */ }
  return framed && location.hostname.endsWith(".hf.space");
})();
