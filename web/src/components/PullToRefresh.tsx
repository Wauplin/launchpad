// Pull to refresh, the one gesture of the phone (Today and Agents; an exception to click only, decided by
// the owner): pulling a list down from its top shows an arrow, and releasing it past the threshold runs `onRefresh`
// while a spinner shows. Place it as the first child of the scrolling element `target`.

import { useEffect, useRef, useState, type RefObject } from "react";
import { Icon } from "./Icon";

const THRESHOLD = 64;
const MAX = 96;

export function PullToRefresh({ target, onRefresh }: { target: RefObject<HTMLElement | null>; onRefresh: () => Promise<unknown> }) {
  const [pull, setPull] = useState(0);
  const [busy, setBusy] = useState(false);
  const refresh = useRef(onRefresh);
  refresh.current = onRefresh;
  useEffect(() => {
    const el = target.current;
    if (!el) return;
    let start: number | null = null;
    let distance = 0;
    const down = (e: TouchEvent) => { start = el.scrollTop <= 0 && e.touches.length === 1 ? e.touches[0].clientY : null; distance = 0; };
    const move = (e: TouchEvent) => {
      if (start === null) return;
      const dy = e.touches[0].clientY - start;
      if (dy <= 0 || el.scrollTop > 0) { start = null; distance = 0; setPull(0); return; }
      distance = Math.min(MAX, dy / 2);
      setPull(distance);
      if (e.cancelable) e.preventDefault();
    };
    const up = () => {
      if (start === null) return;
      start = null;
      const go = distance >= THRESHOLD;
      distance = 0;
      setPull(0);
      if (!go) return;
      setBusy(true);
      void refresh.current().finally(() => setBusy(false));
    };
    el.addEventListener("touchstart", down, { passive: true });
    el.addEventListener("touchmove", move, { passive: false });
    el.addEventListener("touchend", up);
    el.addEventListener("touchcancel", up);
    return () => { el.removeEventListener("touchstart", down); el.removeEventListener("touchmove", move); el.removeEventListener("touchend", up); el.removeEventListener("touchcancel", up); };
  }, [target]);
  if (!pull && !busy) return null;
  const ready = pull >= THRESHOLD;
  return <div className="pointer-events-none sticky top-0 z-10 flex h-0 justify-center" role="status" aria-label={busy ? "Refreshing" : ready ? "Release to refresh" : "Pull to refresh"}>
    <span className="mt-2 grid size-9 place-items-center rounded-full border border-line bg-paper text-ink-2 shadow-[0_4px_12px_-4px_rgba(23,32,43,.35)]" style={{ transform: `translateY(${busy ? 8 : pull - 24}px)` }}>
      {busy ? <span className="lp-spin" /> : <Icon name="chev" size={16} className={ready ? "rotate-180 text-teal-ink" : ""} />}
    </span>
  </div>;
}
