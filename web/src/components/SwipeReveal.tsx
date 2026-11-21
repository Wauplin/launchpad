// Swipe right to reveal one action, Gmail-like but in two steps (a phone gesture decided by the owner, like pull to
// refresh): swiping a row to the right uncovers its action button on the left; only a tap on that button runs it.
// Swiping back, tapping the row or touching another row closes it, and a vertical move scrolls the list as usual.
// The same action stays in the row's ⋯ menu, for keyboards, screen readers and the laptop.

import { useEffect, useRef, useState, type ReactNode } from "react";
import { Icon, type IconName } from "./Icon";

/** Width of the uncovered action, and the distance a move must pass before it counts as a swipe or a scroll. */
const WIDTH = 96;
const SLOP = 10;

/** The open row's close, so opening or touching another row closes it: one open row at a time. */
let closeOpen: (() => void) | null = null;

type Action = { label: string; icon: IconName; disabled?: boolean; title?: string; onClick: () => void };

export function SwipeReveal({ enabled, action, children }: { enabled: boolean; action: Action; children: ReactNode }) {
  const content = useRef<HTMLDivElement>(null);
  const [offset, setOffset] = useState(0);
  const [dragging, setDragging] = useState(false);
  const open = offset >= WIDTH && !dragging;
  const state = useRef({ open: false, suppressClick: false });
  state.current.open = open;

  const close = useRef(() => { setOffset(0); state.current.open = false; });
  useEffect(() => () => { if (closeOpen === close.current) closeOpen = null; }, []);
  // The row lost its action (archived, no longer a thread): put it back in place.
  useEffect(() => { if (!enabled) close.current(); }, [enabled]);

  useEffect(() => {
    const el = content.current;
    if (!el || !enabled) return;
    let start: { x: number; y: number; base: number } | null = null;
    let axis: "x" | "y" | null = null;
    let dx = 0;
    const down = (e: TouchEvent) => {
      if (closeOpen && closeOpen !== close.current) { closeOpen(); closeOpen = null; }
      if (e.touches.length !== 1) { start = null; return; }
      start = { x: e.touches[0].clientX, y: e.touches[0].clientY, base: state.current.open ? WIDTH : 0 };
      axis = null;
      dx = 0;
    };
    const move = (e: TouchEvent) => {
      if (!start) return;
      const x = e.touches[0].clientX - start.x;
      const y = e.touches[0].clientY - start.y;
      if (!axis) {
        if (Math.abs(x) < SLOP && Math.abs(y) < SLOP) return;
        axis = Math.abs(x) > Math.abs(y) ? "x" : "y";
        if (axis === "y") { start = null; return; }
        setDragging(true);
      }
      // A horizontal swipe is ours: the list neither scrolls nor pulls to refresh.
      if (e.cancelable) e.preventDefault();
      e.stopPropagation();
      dx = x - (x > 0 ? SLOP : -SLOP);
      setOffset(Math.max(0, Math.min(WIDTH * 1.25, start.base + dx)));
    };
    const up = () => {
      if (!start || axis !== "x") { start = null; return; }
      const reveal = start.base + dx > WIDTH / 2;
      start = null;
      state.current.suppressClick = true;
      setTimeout(() => { state.current.suppressClick = false; }, 400);
      setDragging(false);
      setOffset(reveal ? WIDTH : 0);
      closeOpen = reveal ? close.current : closeOpen === close.current ? null : closeOpen;
    };
    el.addEventListener("touchstart", down, { passive: true });
    el.addEventListener("touchmove", move, { passive: false });
    el.addEventListener("touchend", up);
    el.addEventListener("touchcancel", up);
    return () => { el.removeEventListener("touchstart", down); el.removeEventListener("touchmove", move); el.removeEventListener("touchend", up); el.removeEventListener("touchcancel", up); };
  }, [enabled]);

  if (!enabled) return <>{children}</>;
  const shown = offset > 0;
  return (
    <div className="relative overflow-hidden">
      <button type="button" tabIndex={open ? 0 : -1} aria-hidden={!open} disabled={action.disabled} title={action.title ?? action.label}
        className="absolute inset-y-0 left-0 flex flex-col items-center justify-center gap-1 bg-teal text-[12px] font-semibold text-white disabled:opacity-60"
        style={{ width: WIDTH, visibility: shown ? "visible" : "hidden" }}
        onClick={() => { close.current(); closeOpen = null; action.onClick(); }}>
        <Icon name={action.icon} size={16} />{action.label}
      </button>
      <div ref={content} className="relative bg-paper" style={{ touchAction: "pan-y", transform: shown ? `translateX(${offset}px)` : undefined, transition: dragging ? "none" : "transform 150ms ease-out" }}
        // A swipe's end is no tap, and a tap on an open row closes it instead of opening the chat.
        onClickCapture={(e) => {
          if (state.current.suppressClick || state.current.open) { e.preventDefault(); e.stopPropagation(); }
          if (!state.current.suppressClick && state.current.open) { close.current(); closeOpen = null; }
        }}>
        {children}
      </div>
    </div>
  );
}
