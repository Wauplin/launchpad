// Small shared phone chrome. Features keep their own actions and state; desktop headers stay separate.
// The shell's top bar is the only title bar of a phone page: a tab page shows its section title there, a detail
// page puts its ‹, title and ⋯ there with `PhoneBar` (a portal into the bar, so the page keeps its own state).
import { createContext, useContext, useLayoutEffect, useRef, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { Icon } from "@/components/Icon";
import { Popover } from "@/components/Popover";
import { useRouter, useRouterState } from "@tanstack/react-router";

/** The shell's top bar: the element a detail page fills, `claim` (returns its release) to hide the section bar, and
 * `end`, the section bar's right side, where a tab page puts its primary button and ⋯ (`PhoneBarEnd`). */
export type PhoneBarSlot = { el: HTMLElement | null; claim: () => () => void; end: HTMLElement | null };
export const PhoneBarContext = createContext<PhoneBarSlot | null>(null);

/** Back for pages that have no single parent (PR page, Files, Terminals): the previous page of this window when
 * there is one, else `fallback` (an installed app opened on a link has no history). */
export function useBack(fallback: () => void) {
  const router = useRouter();
  return () => { if (router.history.canGoBack()) router.history.back(); else fallback(); };
}

/** Back for a page that adds its own history entries (Files: each file is one): returns to the page before the first
 * entry it showed, so the page it was opened from never comes back with a new entry (no PR ↔ Files loop); `fallback`
 * when it was the window's first page. */
export function useLeave(fallback: () => void) {
  const router = useRouter();
  const first = useRef(router.history.location.state.__TSR_index);
  return () => {
    const now = router.history.location.state.__TSR_index;
    if (first.current > 0 && now >= first.current) router.history.go(first.current - now - 1); else fallback();
  };
}

/** A detail page's bar: ‹, the title (up to two lines) and the page's menu or ⓘ in `end`. Render one per page. */
export function PhoneBar({ title, onBack, backLabel = "Back", end }: { title: ReactNode; onBack?: () => void; backLabel?: string; end?: ReactNode }) {
  const slot = useContext(PhoneBarContext);
  useLayoutEffect(() => slot?.claim(), [slot]);
  if (!slot?.el) return null;
  return createPortal(<>
    {onBack ? <button className="iconbtn shrink-0" aria-label={backLabel} title={backLabel} onClick={onBack}><Icon name="chev" size={20} className="rotate-90" /></button> : <span className="w-2 shrink-0" />}
    <h1 className="phone-page-title m-0 flex min-h-11 min-w-0 flex-1 items-center text-[15px] leading-5 font-semibold"><span className="line-clamp-2 break-words">{title}</span></h1>
    {end}
  </>, slot.el);
}

/** A tab page's primary button and ⋯, placed in the section bar next to the status, so the page needs no row of its own. */
export function PhoneBarEnd({ children }: { children: ReactNode }) {
  const slot = useContext(PhoneBarContext);
  return slot?.end ? createPortal(children, slot.end) : null;
}

/** Places of the More tab (the tab shows as current on them). */
export const MORE_PLACES = ["/more", "/schedules", "/projects", "/knowledge", "/terminals", "/settings", "/setup"];

/** The title the phone shell's top bar shows for a path. */
export function pageTitle(path: string): string {
  if (path === "/") return "Today";
  if (path === "/more") return "More";
  if (path === "/chats") return "Chats";
  if (path.startsWith("/agents")) return "Agents";
  if (path.startsWith("/inbox")) return "Inbox";
  if (path.startsWith("/github")) return "GitHub";
  if (path.startsWith("/predrafts")) return "Local PR";
  if (path.startsWith("/terminals")) return "Terminals";
  if (path.startsWith("/settings")) return "Settings";
  if (path.startsWith("/setup")) return "Setup";
  if (path.startsWith("/knowledge")) return "Knowledge";
  if (path.startsWith("/schedules")) return "Schedules";
  if (path.startsWith("/bots") || path.startsWith("/channels")) return "Chats";
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
      {details && <div className="phone-action-details grid min-w-0 gap-3 px-3 py-2">{details}</div>}
    </div>}
  </Popover>;
}

/** A page's header on the phone. With `onBack` (a detail page), the ‹, title and ⋯ go in the shell's bar and only
 * the subtitle and primary button stay in the page. On a tab page the bar keeps the section title and takes the
 * primary button and ⋯ (`PhoneBarEnd`); the page shows a line only for its own name (when it differs) or a subtitle. */
export function PhoneHeader({ title, subtitle, onBack, backLabel = "Back", primary, actions, details }: { title: ReactNode; subtitle?: ReactNode; onBack?: () => void; backLabel?: string; primary?: ReactNode; actions?: ReactNode | ((close: () => void) => ReactNode); details?: ReactNode }) {
  const shellTitle = useRouterState({ select: (s) => pageTitle(s.location.pathname) });
  const menu = (actions || details) && <PhoneActions children={actions} details={onBack ? <><p className="break-words font-semibold">{title}</p>{details}</> : details} />;
  const line = (lead: ReactNode, end: ReactNode) => <header className="phone-page-header flex min-w-0 shrink-0 items-center gap-2 border-b border-line px-3 py-1.5">
    <div className="flex min-w-0 flex-1 flex-wrap items-center gap-x-2 break-words text-[12px] leading-4 text-ink-3">{lead}</div>{primary}{end}
  </header>;
  if (onBack) return <>
    <PhoneBar title={title} onBack={onBack} backLabel={backLabel} end={menu} />
    {(subtitle || primary) && line(subtitle, null)}
  </>;
  const own = title !== shellTitle;
  if (!own && !subtitle) return (primary || menu) ? <PhoneBarEnd>{primary}{menu}</PhoneBarEnd> : null;
  return line(<>{own && <b className="text-[13px] text-ink">{title}</b>}{subtitle}</>, menu);
}
