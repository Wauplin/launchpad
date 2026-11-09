// The sidebar while Settings is open (laptop): it replaces the normal sidebar, so Settings has one level of navigation.
// Back to Launchpad (top, the only way out) returns to the page that was open before Settings.

import { Link, useRouter, useRouterState } from "@tanstack/react-router";
import { useEffect } from "react";
import { Icon } from "@/components/Icon";
import { useScope } from "@/lib/workspace";
import { useTheme } from "@/stores/theme";
import { settingsPages, useSettingsSearch, type NavEntry } from "@/features/settings/nav";

export const inSettings = (pathname: string) => pathname === "/settings" || pathname.startsWith("/settings/");

/** The last page outside Settings in this window, where closing Settings goes back to. */
let lastPlace = "/";

/** Records the window's last page outside Settings (the shell calls it). */
export function useRememberPlace() {
  const href = useRouterState({ select: (s) => s.location.href });
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  useEffect(() => { if (!inSettings(pathname)) lastPlace = href; }, [href, pathname]);
}

function SettingsLink({ e, detail }: { e: NavEntry; detail?: string }) {
  return (
    <Link to={e.to} activeOptions={{ exact: !!e.exact, includeSearch: false }} className="flex min-h-[30px] flex-none items-center gap-[9px] rounded-[7px] px-[9px] text-[13px] text-ink-2 no-underline hover:bg-sunk hover:text-ink" activeProps={{ className: "!bg-paper !text-ink font-semibold shadow-[inset_0_0_0_1px_var(--color-line)]", "aria-current": "page" } as never}>
      <Icon name={e.icon} size={14} className="flex-none" />
      <span className="min-w-0 truncate">{e.label}</span>
      {detail && <span className="ml-auto min-w-0 truncate text-[12px] font-normal text-ink-3">{detail}</span>}
    </Link>
  );
}

export function SettingsSidebar() {
  const router = useRouter();
  const { theme, toggle } = useTheme();
  const workspace = useScope().workspace;
  const query = useSettingsSearch((s) => s.query);
  const setQuery = useSettingsSearch((s) => s.set);
  const pages = settingsPages(query);
  const close = () => router.history.push(lastPlace);
  return (
    <aside className="flex w-[200px] flex-none flex-col border-r border-line bg-chrome" aria-label="Settings">
      <div className="flex-none border-b border-line-2 px-2 py-1.5">
        <button type="button" className="flex h-[31px] w-full min-w-0 items-center gap-1.5 rounded-[7px] px-[9px] text-left text-ink-2 hover:bg-sunk hover:text-ink" title="Close Settings and go back to the page you were on" onClick={close}>
          <Icon name="chev" size={12} className="flex-none rotate-90" />
          <span className="min-w-0 flex-1 truncate">Back to Launchpad</span>
        </button>
      </div>
      <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-2 pt-2.5 pb-2">
        <div className="flex items-center px-[9px] text-[15px] font-semibold text-ink">Settings</div>
        <label className="flex h-8 flex-none items-center gap-1.5 rounded-[7px] border border-line bg-paper px-2 text-ink-3 focus-within:border-teal">
          <Icon name="search" size={13} />
          <input type="search" size={1} aria-label="Search settings" placeholder="Search settings" className="min-w-0 flex-1 bg-transparent text-[13px] text-ink outline-none" value={query} onChange={(e) => setQuery(e.target.value)} />
        </label>
        <nav className="flex flex-none flex-col gap-3" aria-label="Settings pages">
          {pages.workspace.length > 0 && <div className="flex flex-col gap-px" title="Settings of the workspace shown in this window">
            {pages.workspace.map((e) => <SettingsLink key={e.to} e={e} detail={workspace.name} />)}
          </div>}
          {pages.groups.length > 0 && <div className={`flex flex-col gap-3 ${pages.workspace.length > 0 ? "border-t border-line-2 pt-3" : ""}`}>
            <div className="px-[9px] text-[13px] font-semibold text-ink" title="Shared by every workspace">Launchpad</div>
            {pages.groups.map((g) => (
              <div key={g.group} className="flex flex-col gap-px">
                <div className="seclabel px-[9px] pb-1">{g.group}</div>
                {g.items.map((e) => <SettingsLink key={e.to} e={e} />)}
              </div>
            ))}
          </div>}
          {!pages.workspace.length && !pages.groups.length && <div className="px-[9px] text-[12px] text-ink-3">No settings page matches “{query.trim()}”.</div>}
        </nav>
      </div>
      <div className="flex flex-none items-center justify-end gap-1 border-t border-line-2 px-2 pt-1.5 pb-2.5">
        <button type="button" className="flex size-[31px] flex-none items-center justify-center rounded-[7px] text-ink-2 hover:bg-sunk" aria-label="Dark theme" aria-pressed={theme === "dark"} title={`Switch to ${theme === "light" ? "dark" : "light"} theme on this device`} onClick={toggle}>
          <Icon name={theme === "dark" ? "moon" : "sun"} size={15} />
        </button>
      </div>
    </aside>
  );
}
