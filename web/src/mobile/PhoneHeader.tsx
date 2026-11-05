// Small shared phone chrome. Features keep their own actions and state; desktop headers stay separate.
import type { ReactNode } from "react";
import { Icon } from "@/components/Icon";
import { Popover } from "@/components/Popover";
import { useRouterState } from "@tanstack/react-router";

/** The title the phone shell's top bar shows for a path. */
export function pageTitle(path: string): string {
  if (path.startsWith("/now")) return "Now";
  if (path.startsWith("/agents")) return "Agents";
  if (path.startsWith("/review")) return "Review";
  if (path.startsWith("/inbox") || path === "/") return "Inbox";
  if (path.startsWith("/outbox")) return "Outbox";
  if (path.startsWith("/github")) return "GitHub";
  if (path.startsWith("/predrafts")) return "Local PR";
  if (path.startsWith("/proposals")) return "Proposals";
  if (path.startsWith("/terminals")) return "Terminals";
  if (path.startsWith("/settings")) return "Settings";
  if (path.startsWith("/setup")) return "Setup";
  if (path.startsWith("/knowledge")) return "Knowledge";
  if (path.startsWith("/schedules")) return "Schedules";
  if (path.startsWith("/bots")) return "Bots";
  if (path.startsWith("/projects")) return path.endsWith("/code") ? "Files" : "Folders";
  return "Launchpad";
}

export function PhoneActions({ label = "Actions & details", children, details, closeOnAction = true }: { label?: string; children?: ReactNode | ((close: () => void) => ReactNode); details?: ReactNode; closeOnAction?: boolean }) {
  return <Popover viewport keepMounted align="right" width={280} label={label}
    trigger={({ open, toggle }) => <button className="iconbtn shrink-0" aria-label={label} aria-haspopup="dialog" aria-expanded={open} title={label} onClick={toggle}><Icon name="more" size={20} /></button>}
  >
    {(close) => <div className="phone-action-content grid min-w-0 gap-2">
      {children && <div className="phone-action-buttons grid min-w-0" onClickCapture={(e) => {
        const button = (e.target as Element).closest("button, a");
        // Pickers and multi-step controls retain their state and manage their own dismissal.
        if (closeOnAction && button && !button.hasAttribute("aria-expanded") && !button.hasAttribute("disabled")) queueMicrotask(close);
      }}>{typeof children === "function" ? children(close) : children}</div>}
      {details && <div className="phone-action-details grid min-w-0 gap-3 px-3 py-2 text-sm">{details}</div>}
    </div>}
  </Popover>;
}

/** A page's own header under the shell's top bar. A title the top bar already shows is not repeated: only the
 * subtitle, primary button and actions remain, and nothing at all when there are none. */
export function PhoneHeader({ title, subtitle, onBack, backLabel = "Back", primary, actions, details }: { title: ReactNode; subtitle?: ReactNode; onBack?: () => void; backLabel?: string; primary?: ReactNode; actions?: ReactNode; details?: ReactNode }) {
  const shellTitle = useRouterState({ select: (s) => pageTitle(s.location.pathname) });
  if (!onBack && title === shellTitle) {
    if (!subtitle && !primary && !actions && !details) return null;
    return <header className="phone-page-header flex min-w-0 shrink-0 items-center gap-2 border-b border-line px-3 py-1.5">
      <div className="min-w-0 flex-1 break-words text-xs leading-4 text-ink-3">{subtitle}</div>{primary}
      {(actions || details) && <PhoneActions children={actions} details={details} />}
    </header>;
  }
  return <header className="phone-page-header shrink-0 border-b border-line px-3 pt-1.5 pb-2">
    <div className="flex min-w-0 items-start gap-1">
      {onBack && <button className="iconbtn shrink-0" aria-label={backLabel} title={backLabel} onClick={onBack}><Icon name="chev" size={20} className="rotate-90" /></button>}
      <h1 className="phone-page-title flex min-h-11 min-w-0 flex-1 items-center text-[15px] leading-5 font-semibold"><span className="line-clamp-2 break-words">{title}</span></h1>
      {(actions || details) && <PhoneActions children={actions} details={<><p className="break-words font-semibold">{title}</p>{details}</>} />}
    </div>
    {(subtitle || primary) && <div className="flex min-w-0 items-center gap-2 pt-1"><div className="min-w-0 flex-1 break-words text-xs leading-4 text-ink-3">{subtitle}</div>{primary}</div>}
  </header>;
}
