// A main area and a side panel on the right, split by a line the owner drags (or moves with the arrow keys when
// focused). The panel width is device-local (browser storage, per `storageKey`; the demo keeps it in memory) and
// stays between `min` px and `maxRatio` of the view, so the main area never gets narrower than the panel allows.
import { useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { useZoomScale } from "@/lib/zoom";

const read = (key: string, fallback: number) => {
  const value = Number(window.__lpDemo ? NaN : localStorage.getItem(key));
  return Number.isFinite(value) && value > 0 ? value : fallback;
};

export function SplitView({ storageKey, main, side, initial = 400, min = 280, maxRatio = 0.65, sideLabel }: {
  storageKey: string; main: ReactNode; side: ReactNode; initial?: number; min?: number; maxRatio?: number; sideLabel: string;
}) {
  const scale = useZoomScale();
  const [width, setWidth] = useState(() => read(storageKey, initial));
  const [total, setTotal] = useState(0);
  const root = useRef<HTMLDivElement>(null);
  const drag = useRef<{ pointer: number; x: number; width: number } | null>(null);
  useLayoutEffect(() => {
    const el = root.current;
    if (!el) return;
    const measure = () => setTotal(el.getBoundingClientRect().width / scale);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, [scale]);
  const maximum = total ? Math.max(min, total * maxRatio) : 10000;
  const clamp = (value: number) => Math.round(Math.max(min, Math.min(maximum, value)));
  const shown = clamp(width);
  const set = (value: number) => {
    const next = clamp(value);
    setWidth(next);
    if (!window.__lpDemo) localStorage.setItem(storageKey, String(next));
  };
  return (
    <div ref={root} className="flex min-h-0 min-w-0 flex-1">
      <div className="flex min-h-0 min-w-0 flex-1 flex-col">{main}</div>
      <div
        role="separator"
        aria-label={`Resize ${sideLabel}`}
        aria-orientation="vertical"
        aria-valuemin={min}
        aria-valuemax={Math.round(maximum)}
        aria-valuenow={shown}
        tabIndex={0}
        title="Drag to resize (Arrow Left/Right when focused, double-click to reset)"
        className="group relative z-10 -mx-[3px] flex w-[7px] flex-none cursor-col-resize touch-none select-none justify-center focus-visible:outline-none"
        onPointerDown={(event) => {
          if (event.button !== 0 || !event.isPrimary) return;
          event.preventDefault();
          event.currentTarget.focus();
          event.currentTarget.setPointerCapture(event.pointerId);
          drag.current = { pointer: event.pointerId, x: event.clientX, width: shown };
        }}
        onPointerMove={(event) => {
          const start = drag.current;
          if (start?.pointer === event.pointerId) set(start.width + (start.x - event.clientX) / scale);
        }}
        onPointerUp={(event) => {
          if (drag.current?.pointer !== event.pointerId) return;
          drag.current = null;
          event.currentTarget.releasePointerCapture(event.pointerId);
        }}
        onPointerCancel={() => { drag.current = null; }}
        onLostPointerCapture={() => { drag.current = null; }}
        onDoubleClick={() => set(initial)}
        onKeyDown={(event) => {
          const step = event.shiftKey ? 50 : 10;
          if (event.key === "ArrowLeft") set(shown + step);
          else if (event.key === "ArrowRight") set(shown - step);
          else return;
          event.preventDefault();
        }}
      >
        <span className="w-px bg-line group-hover:w-[3px] group-hover:bg-teal group-focus-visible:w-[3px] group-focus-visible:bg-teal" />
      </div>
      <aside className="flex min-h-0 min-w-0 flex-none flex-col" style={{ width: shown }} aria-label={sideLabel}>{side}</aside>
    </div>
  );
}
