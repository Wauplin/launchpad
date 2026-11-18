// A main area and a side panel on the right, split by a line the owner drags (`ResizeHandle`). The panel width is
// device-local (browser storage, per `storageKey`; the demo keeps it in memory) and stays between `min` px and `maxRatio` of the view, so the main area never gets narrower than the panel allows.
// Without `side` (a closed drawer) only the main area shows, and it stays mounted when the panel opens or closes.
import { useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { useZoomScale } from "@/lib/zoom";
import { ResizeHandle } from "./ResizeHandle";

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
      {side && <>
      <ResizeHandle label={sideLabel} value={shown} min={min} max={maximum} side="right" onChange={set} onReset={() => set(initial)} />
      <aside className="flex min-h-0 min-w-0 flex-none flex-col" style={{ width: shown }} aria-label={sideLabel}>{side}</aside>
      </>}
    </div>
  );
}
