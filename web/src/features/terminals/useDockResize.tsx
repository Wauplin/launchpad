import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useZoomScale } from "@/lib/zoom";

/** Resize in layout pixels, accounting for the desktop's CSS zoom. Persist only on release. */
export function useDockResize(savedHeight: number, save: (dock: { height: number }) => void, enabled: boolean) {
  const ref = useRef<HTMLElement>(null);
  const scale = useZoomScale();
  const [available, setAvailable] = useState(0);
  const [preview, setPreview] = useState<number | null>(null);
  const drag = useRef<{ pointer: number; y: number; height: number; current: number } | null>(null);
  useLayoutEffect(() => {
    const parent = ref.current?.parentElement;
    if (!enabled || !parent) return;
    const observer = new ResizeObserver(() => setAvailable(parent.getBoundingClientRect().height / scale));
    observer.observe(parent);
    return () => observer.disconnect();
  }, [enabled, scale]);
  useEffect(() => {
    if (!enabled) { drag.current = null; setPreview(null); }
  }, [enabled]);

  // Leave room for the page above the dock, and allow small windows to shrink below the usual minimum.
  const max = available ? Math.max(0, available - Math.min(120, available / 2)) : savedHeight;
  const min = Math.min(160, max);
  const clamp = (value: number) => Math.round(Math.min(max, Math.max(min, value)));
  const height = clamp(preview ?? savedHeight);
  const finish = () => {
    if (!drag.current) return;
    save({ height: clamp(drag.current.current) });
    drag.current = null;
    setPreview(null);
  };
  const handle = (
    <div
      role="separator"
      aria-label="Resize terminal dock"
      aria-orientation="horizontal"
      aria-valuemin={min}
      aria-valuemax={max}
      aria-valuenow={height}
      aria-valuetext={`${height} pixels`}
      tabIndex={0}
      title="Drag to resize terminal"
      className="group absolute -top-1 left-0 z-10 flex h-2 w-full touch-none cursor-row-resize select-none items-center justify-center hover:bg-teal/20 focus-visible:bg-teal/20 focus-visible:outline-none"
      onPointerDown={(event) => {
        if (event.button !== 0 || drag.current) return;
        event.preventDefault();
        event.currentTarget.setPointerCapture(event.pointerId);
        drag.current = { pointer: event.pointerId, y: event.clientY, height, current: height };
      }}
      onPointerMove={(event) => {
        const start = drag.current;
        if (!start || event.pointerId !== start.pointer) return;
        start.current = clamp(start.height + (start.y - event.clientY) / scale);
        setPreview(start.current);
      }}
      onPointerUp={finish}
      onPointerCancel={finish}
      onLostPointerCapture={finish}
      onKeyDown={(event) => {
        let next: number;
        const step = event.shiftKey ? 50 : 10;
        if (event.key === "ArrowUp") next = height + step;
        else if (event.key === "ArrowDown") next = height - step;
        else if (event.key === "Home") next = min;
        else if (event.key === "End") next = max;
        else return;
        event.preventDefault();
        save({ height: clamp(next) });
      }}
    >
      <span className="h-0.5 w-10 rounded bg-line group-hover:bg-teal group-focus-visible:bg-teal" />
    </div>
  );
  return { ref, height, handle };
}
