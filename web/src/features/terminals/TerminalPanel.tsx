// Desktop dock resizing uses CSS pixels; pointer coordinates include the app's root zoom. The height is a
// share of the area above the status bar (default 35%, never more than half), remembered per window.
import { useLayoutEffect, useRef, useState, type ReactNode } from "react";

const KEY = "launchpad-dock-height";
const DEFAULT_RATIO = 0.35;
const MAX_RATIO = 0.5;

function storedRatio() {
  const value = Number(sessionStorage.getItem(KEY));
  return Number.isFinite(value) && value > 0 && value <= MAX_RATIO ? value : DEFAULT_RATIO;
}

export function TerminalPanel({ phone, open, scale, children }: {
  phone: boolean;
  open: boolean;
  scale: number;
  children: ReactNode;
}) {
  const [ratio, setRatio] = useState(storedRatio);
  const [area, setArea] = useState(0);
  const panel = useRef<HTMLElement>(null);
  const drag = useRef<{ pointer: number; y: number; height: number } | null>(null);
  const [maximum, setMaximum] = useState(10000);
  useLayoutEffect(() => {
    const parent = panel.current?.parentElement;
    if (phone || !parent) return;
    // Never more than half of the area, even after shrinking the window or changing zoom.
    const measure = () => {
      const height = parent.getBoundingClientRect().height / scale;
      setArea(height);
      setMaximum(Math.max(0, height * MAX_RATIO));
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(parent);
    return () => observer.disconnect();
  }, [phone, scale]);
  useLayoutEffect(() => { drag.current = null; }, [phone, open, scale]);

  const minimum = Math.min(120, maximum);
  const clamp = (value: number) => Math.max(minimum, Math.min(maximum, value));
  const visibleHeight = clamp(area ? ratio * area : 320);
  const onHeight = (height: number) => {
    if (!area) return;
    const next = Math.min(MAX_RATIO, height / area);
    setRatio(next);
    sessionStorage.setItem(KEY, String(next));
  };
  return (
    <section ref={panel} className={`flex flex-col border-t border-line bg-paper ${phone ? "min-h-0 flex-1" : "min-h-0 flex-none"}`} style={{ height: phone ? undefined : visibleHeight, display: open ? undefined : "none" }} aria-label="Terminals">
      {!phone && <div
        role="separator"
        aria-label="Resize terminal"
        aria-orientation="horizontal"
        aria-valuemin={Math.round(minimum)}
        aria-valuemax={Math.round(maximum)}
        aria-valuenow={Math.round(visibleHeight)}
        aria-valuetext={`${Math.round(visibleHeight)} pixels`}
        tabIndex={0}
        title="Drag to resize terminal (Arrow Up/Down when focused)"
        className="group flex h-2 flex-none touch-none select-none items-center justify-center cursor-row-resize hover:bg-select focus-visible:bg-select focus-visible:outline-none"
        onPointerDown={(event) => {
          if (event.button !== 0 || !event.isPrimary) return;
          event.preventDefault();
          event.currentTarget.focus();
          event.currentTarget.setPointerCapture(event.pointerId);
          drag.current = { pointer: event.pointerId, y: event.clientY, height: visibleHeight };
        }}
        onPointerMove={(event) => {
          const start = drag.current;
          if (start?.pointer === event.pointerId) onHeight(clamp(start.height + (start.y - event.clientY) / scale));
        }}
        onPointerUp={(event) => {
          if (drag.current?.pointer !== event.pointerId) return;
          drag.current = null;
          event.currentTarget.releasePointerCapture(event.pointerId);
        }}
        onPointerCancel={() => { drag.current = null; }}
        onLostPointerCapture={() => { drag.current = null; }}
        onKeyDown={(event) => {
          let next: number;
          switch (event.key) {
            case "ArrowUp": next = visibleHeight + (event.shiftKey ? 50 : 10); break;
            case "ArrowDown": next = visibleHeight - (event.shiftKey ? 50 : 10); break;
            case "Home": next = minimum; break;
            case "End": next = maximum; break;
            default: return;
          }
          event.preventDefault();
          onHeight(clamp(next));
        }}
      >
        <span className="h-0.5 w-8 rounded bg-line group-hover:bg-teal group-focus-visible:bg-teal" />
      </div>}
      {children}
    </section>
  );
}
