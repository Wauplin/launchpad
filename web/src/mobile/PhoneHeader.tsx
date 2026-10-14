// Small shared phone chrome. Features keep their own actions and state; desktop headers stay separate.
import type { ReactNode } from "react";
import { Icon } from "@/components/Icon";
import { Popover } from "@/components/Popover";

export function PhoneActions({ label = "Actions & details", children, details, closeOnAction = true }: { label?: string; children?: ReactNode; details?: ReactNode; closeOnAction?: boolean }) {
  return <Popover viewport keepMounted align="right" width={280} label={label}
    trigger={({ open, toggle }) => <button className={`iconbtn shrink-0 ${open ? "bg-select!" : ""}`} aria-label={label} aria-haspopup="dialog" aria-expanded={open} title={label} onClick={toggle}><span aria-hidden="true" className="text-xl leading-none">⋯</span></button>}
  >
    {(close) => <div className="phone-action-content grid min-w-0 gap-2">
      {children && <div className="phone-action-buttons grid min-w-0" onClickCapture={(e) => {
        const button = (e.target as Element).closest("button, a");
        // Pickers and multi-step controls retain their state and manage their own dismissal.
        if (closeOnAction && button && !button.hasAttribute("aria-expanded") && !button.hasAttribute("disabled")) queueMicrotask(close);
      }}>{children}</div>}
      {details && <div className="phone-action-details grid min-w-0 gap-3 px-3 py-2 text-sm">{details}</div>}
    </div>}
  </Popover>;
}

export function PhoneHeader({ title, subtitle, onBack, backLabel = "Back", primary, actions, details }: { title: ReactNode; subtitle?: ReactNode; onBack?: () => void; backLabel?: string; primary?: ReactNode; actions?: ReactNode; details?: ReactNode }) {
  return <header className="phone-page-header shrink-0 border-b border-line px-3 pt-1.5 pb-2">
    <div className="flex min-w-0 items-start gap-1">
      {onBack && <button className="iconbtn shrink-0" aria-label={backLabel} title={backLabel} onClick={onBack}><Icon name="chev" size={20} className="rotate-90" /></button>}
      <h1 className="phone-page-title flex min-h-11 min-w-0 flex-1 items-center text-[15px] leading-5 font-semibold"><span className="line-clamp-2 break-words">{title}</span></h1>
      {(actions || details) && <PhoneActions children={actions} details={<><p className="break-words font-semibold">{title}</p>{details}</>} />}
    </div>
    {(subtitle || primary) && <div className="flex min-w-0 items-center gap-2 pt-1"><div className="min-w-0 flex-1 break-words text-xs leading-4 text-ink-3">{subtitle}</div>{primary}</div>}
  </header>;
}
