// A panel anchored under (or above) its trigger. Closes on a click outside or Escape.

import { useEffect, useRef, useState, type ReactNode } from "react";

export function Popover({
  trigger,
  children,
  align = "left",
  up = false,
  width,
  className = "",
}: {
  trigger: (props: { open: boolean; toggle: () => void }) => ReactNode;
  children: (close: () => void) => ReactNode;
  align?: "left" | "right";
  up?: boolean;
  width?: number;
  className?: string;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);
  return (
    <div ref={ref} className={`relative ${className}`}>
      {trigger({ open, toggle: () => setOpen((o) => !o) })}
      {open && (
        <div
          className={`absolute z-30 grid max-h-80 gap-px overflow-y-auto rounded-[9px] border border-line bg-paper p-[5px] shadow-[0_12px_28px_-10px_rgba(23,32,43,.35)] ${
            up ? "bottom-full mb-1" : "top-full mt-1"
          } ${align === "right" ? "right-0" : "left-0"}`}
          style={{ width: width ?? undefined, minWidth: width ? undefined : 230 }}
        >
          {children(() => setOpen(false))}
        </div>
      )}
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
      className={`grid grid-cols-[16px_1fr] gap-x-[9px] gap-y-px rounded-md px-[9px] py-[7px] text-left text-[12.5px] hover:bg-select disabled:opacity-50 ${
        active ? "bg-select" : ""
      }`}
    >
      <span className="flex items-center">{icon}</span>
      <span className="min-w-0 truncate">{label}</span>
      {detail && <span className="col-start-2 truncate text-[11.5px] text-ink-3">{detail}</span>}
    </button>
  );
}
