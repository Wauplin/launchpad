// The line between two laptop columns, which the owner drags (or moves with the arrow keys when focused) to resize
// the column on its `side`; a double-click gives the column its default width. The hit area is 7 px wide around a
// 1 px line, so it takes no layout room. While dragging, the pointer is captured and `lp-resizing` on the root keeps
// the resize cursor everywhere, stops text selection and keeps frames (previews, sketches) from taking the pointer.
// `useColumn` is the same handle for the named columns of `lib/columns.ts`, whose widths every page shares.
import { useEffect, useRef, useState } from "react";
import { clampColumn, columnMax, COLUMNS, type ColumnName } from "@/lib/columns";
import { useZoomScale } from "@/lib/zoom";
import { useColumns } from "@/stores/columns";

const resizing = (on: boolean) => { document.documentElement.classList.toggle("lp-resizing", on); };

export function ResizeHandle({ label, value, min, max, side, onChange, onReset }: {
  label: string; value: number; min: number; max: number;
  /** Where the resized column is: dragging towards the other side widens it. */
  side: "left" | "right";
  onChange: (width: number) => void; onReset: () => void;
}) {
  const scale = useZoomScale();
  const drag = useRef<{ pointer: number; x: number; width: number } | null>(null);
  const sign = side === "left" ? 1 : -1;
  const end = () => { drag.current = null; resizing(false); };
  useEffect(() => () => resizing(false), []);
  return (
    <div
      role="separator"
      aria-label={`Resize ${label}`}
      aria-orientation="vertical"
      aria-valuemin={Math.round(min)}
      aria-valuemax={Math.round(max)}
      aria-valuenow={Math.round(value)}
      tabIndex={0}
      title="Drag to resize (Arrow Left/Right when focused, double-click to reset)"
      className="group relative z-10 -mx-[3px] flex w-[7px] flex-none cursor-col-resize touch-none select-none justify-center focus-visible:outline-none"
      onPointerDown={(event) => {
        if (event.button !== 0 || !event.isPrimary) return;
        event.preventDefault();
        event.currentTarget.focus();
        event.currentTarget.setPointerCapture(event.pointerId);
        drag.current = { pointer: event.pointerId, x: event.clientX, width: value };
        resizing(true);
      }}
      onPointerMove={(event) => {
        const start = drag.current;
        if (start?.pointer === event.pointerId) onChange(start.width + sign * (event.clientX - start.x) / scale);
      }}
      onPointerUp={(event) => {
        if (drag.current?.pointer !== event.pointerId) return;
        end();
        event.currentTarget.releasePointerCapture(event.pointerId);
      }}
      onPointerCancel={end}
      onLostPointerCapture={end}
      onDoubleClick={onReset}
      onKeyDown={(event) => {
        const step = event.shiftKey ? 50 : 10;
        if (event.key === "ArrowRight") onChange(value + sign * step);
        else if (event.key === "ArrowLeft") onChange(value - sign * step);
        else return;
        event.preventDefault();
      }}
    >
      <span className="w-px bg-line group-hover:w-[3px] group-hover:bg-teal group-focus-visible:w-[3px] group-focus-visible:bg-teal group-active:w-[3px] group-active:bg-teal" />
    </div>
  );
}

/** The window's width in layout px (the root's zoom applied), followed on resize. */
function useViewport() {
  const scale = useZoomScale();
  const [width, setWidth] = useState(() => window.innerWidth);
  useEffect(() => {
    const measure = () => setWidth(window.innerWidth);
    window.addEventListener("resize", measure);
    return () => window.removeEventListener("resize", measure);
  }, []);
  return width / scale;
}

/** A named column's width and the handle to put right after it (the column then has no right border of its own). */
export function useColumn(name: ColumnName, label: string) {
  const spec = COLUMNS[name];
  const viewport = useViewport();
  const stored = useColumns((s) => s.widths[name]);
  const set = useColumns((s) => s.set);
  const width = clampColumn(spec, stored, viewport);
  const handle = <ResizeHandle label={label} value={width} min={spec.min} max={columnMax(spec, viewport)} side="left"
    onChange={(next) => set(name, clampColumn(spec, next, viewport))} onReset={() => set(name, null)} />;
  return { width, handle };
}
