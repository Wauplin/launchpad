// User activity in this window: pointer, key, wheel and touch input. A focused window only "watches" a
// session (marks it seen, suppresses its notification) when someone used it in the last minute, so an
// unattended laptop does not take Needs you and notifications away from another device.

import { useEffect, useState } from "react";

/** Input within this delay counts as someone in front of the window (the daemon uses the same value). */
export const ACTIVE_MS = 60_000;

let lastInputAt: number | null = null;
const listeners = new Set<() => void>();

function input() {
  lastInputAt = Date.now();
  listeners.forEach((fn) => fn());
}

if (typeof window !== "undefined") {
  for (const type of ["pointermove", "pointerdown", "keydown", "wheel", "touchstart"])
    window.addEventListener(type, input, { capture: true, passive: true });
}

/** Milliseconds since the last input in this window, or null before any input. */
export function idleMs(): number | null {
  return lastInputAt === null ? null : Date.now() - lastInputAt;
}

export function isUserActive(): boolean {
  const idle = idleMs();
  return idle !== null && idle < ACTIVE_MS;
}

/** Called on every input event (keep listeners cheap or throttle them). Returns the unsubscribe function. */
export function onUserInput(fn: () => void): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

/** Whether this window received input in the last minute; re-renders only when that changes. */
export function useUserActive(): boolean {
  const [active, setActive] = useState(isUserActive);
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    let on = false;
    // While active, one timer re-checks at the expiry of the latest input; input only matters when idle.
    const check = () => {
      clearTimeout(timer);
      const idle = idleMs();
      on = idle !== null && idle < ACTIVE_MS;
      setActive(on);
      if (idle !== null && on) timer = setTimeout(check, ACTIVE_MS - idle + 50);
    };
    check();
    const off = onUserInput(() => { if (!on) check(); });
    return () => { clearTimeout(timer); off(); };
  }, []);
  return active;
}
