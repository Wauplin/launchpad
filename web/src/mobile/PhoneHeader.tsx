// Small shared phone chrome. Features keep their own actions and state; desktop headers stay separate.
import { useState, type ReactNode } from "react";
import { Icon } from "@/components/Icon";
import { Sheet } from "./Sheet";

export function PhoneActions({ label = "Actions & details", children, details, closeOnAction = true }: { label?: string; children?: ReactNode; details?: ReactNode; closeOnAction?: boolean }) {
  const [open, setOpen] = useState(false);
  return <>
    <button className="iconbtn shrink-0" aria-label={label} aria-expanded={open} title={label} onClick={() => setOpen(true)}><span aria-hidden="true" className="text-xl leading-none">⋯</span></button>
    <div hidden={!open}><Sheet title={label} onClose={() => setOpen(false)}>
      <div className="phone-action-sheet grid gap-3 p-4">
        {children && <div className="phone-action-buttons flex flex-wrap items-center gap-2" onClickCapture={(e) => {
          const button = (e.target as Element).closest("button, a");
          // Let pickers and multi-step details retain their own controls. Navigation/launch/refresh closes the sheet.
          if (closeOnAction && button && !button.hasAttribute("aria-expanded") && !button.hasAttribute("disabled")) queueMicrotask(() => setOpen(false));
        }}>{children}</div>}
        {details}
      </div>
    </Sheet></div>
  </>;
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
