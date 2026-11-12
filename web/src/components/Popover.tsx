// A panel anchored under (or above) its trigger. Closes on a click outside or Escape.

import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { useZoomScale } from "@/lib/zoom";
import { useBackClose } from "@/mobile/back";
import { placePanel } from "./popoverPlace";

export function Popover({
  trigger,
  children,
  align = "left",
  up = false,
  width,
  className = "",
  viewport = false,
  keepMounted = false,
  label,
}: {
  trigger: (props: { open: boolean; toggle: () => void }) => ReactNode;
  children: (close: () => void) => ReactNode;
  align?: "left" | "right";
  up?: boolean;
  width?: number | string;
  className?: string;
  // Portal outside scrolling containers and fit the visual viewport, including desktop zoom.
  viewport?: boolean;
  keepMounted?: boolean;
  label?: string;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState<CSSProperties>({});
  const scale = useZoomScale();
  // On the phone, an open menu is a history entry: Android back closes it.
  useBackClose(open, () => setOpen(false));
  useLayoutEffect(() => {
    if (!open || !viewport) return;
    const place = () => {
      const anchor = ref.current?.getBoundingClientRect();
      const panel = panelRef.current;
      if (!anchor || !panel) return;
      const view = window.visualViewport;
      const next = placePanel(anchor, { x: view?.offsetLeft ?? 0, y: view?.offsetTop ?? 0, w: view?.width ?? window.innerWidth, h: view?.height ?? window.innerHeight }, { width: panel.getBoundingClientRect().width, scrollHeight: panel.scrollHeight }, align, scale);
      if (next) setPosition(next);
      else setOpen(false);
    };
    place();
    const observer = new ResizeObserver(place);
    if (panelRef.current) observer.observe(panelRef.current);
    window.addEventListener("resize", place);
    document.addEventListener("scroll", place, true);
    window.visualViewport?.addEventListener("resize", place);
    window.visualViewport?.addEventListener("scroll", place);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", place);
      document.removeEventListener("scroll", place, true);
      window.visualViewport?.removeEventListener("resize", place);
      window.visualViewport?.removeEventListener("scroll", place);
    };
  }, [open, viewport, scale, align]);
  useEffect(() => {
    if (!open) return;
    const onDown = (e: Event) => {
      if (ref.current && !ref.current.contains(e.target as Node) && !panelRef.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      setOpen(false);
      if (viewport) ref.current?.querySelector("button")?.focus();
    };
    if (viewport) panelRef.current?.focus({ preventScroll: true });
    const outsideEvent = viewport ? "pointerdown" : "mousedown";
    document.addEventListener(outsideEvent, onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener(outsideEvent, onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open, viewport]);
  const panel = (open || keepMounted) && (
    <div
      ref={panelRef}
      role={label ? "dialog" : undefined}
      aria-label={label}
      tabIndex={viewport ? -1 : undefined}
      className={`${viewport ? "phone-action-popover fixed z-50" : `absolute z-30 ${up ? "bottom-full mb-1" : "top-full mt-1"} ${align === "right" ? "right-0" : "left-0"}`} grid max-h-80 gap-px overflow-y-auto rounded-card border border-line bg-paper p-[5px] shadow-[0_12px_28px_-10px_rgba(23,32,43,.35)]`}
      style={{ width: width ?? undefined, minWidth: width ? undefined : 230, ...(viewport ? position : {}), ...(!open ? { display: "none" } : {}) }}
    >
      {children(() => setOpen(false))}
    </div>
  );
  return (
    <div ref={ref} className={`relative ${className}`}>
      {trigger({ open, toggle: () => setOpen((o) => !o) })}
      {viewport ? createPortal(panel, document.body) : panel}
    </div>
  );
}

export function MenuItem({
  onClick,
  icon,
  label,
  detail,
  active,
  disabled,
}: {
  onClick: () => void;
  icon?: ReactNode;
  label: ReactNode;
  detail?: ReactNode;
  active?: boolean;
  disabled?: boolean;
}) {
  return (
    <button
      disabled={disabled}
      onClick={onClick}
      className={`grid grid-cols-[16px_minmax(0,1fr)] content-center items-center gap-x-2 gap-y-px rounded-control px-[9px] py-[7px] text-left text-[13px] hover:bg-select disabled:opacity-50 ${
        active ? "bg-select" : ""
      }`}
    >
      <span className="flex items-center justify-center">{icon}</span>
      <span className="min-w-0 truncate">{label}</span>
      {detail && <span className="col-start-2 truncate text-[12px] text-ink-3">{detail}</span>}
    </button>
  );
}
