import { Link } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { Icon, type IconName } from "@/components/Icon";

function NavItem({ to, icon, children, end }: { to: string; icon: IconName; children: ReactNode; end?: ReactNode }) {
  return (
    <Link
      to={to}
      className="flex h-[31px] items-center gap-[9px] rounded-[7px] px-[9px] text-ink-2 no-underline hover:bg-sunk"
      activeProps={{ className: "!bg-paper !text-ink font-semibold shadow-[inset_0_0_0_1px_var(--color-line)]" }}
      activeOptions={{ exact: to === "/" }}
    >
      <Icon name={icon} />
      {children}
      {end && <span className="ml-auto">{end}</span>}
    </Link>
  );
}

export function SideLabel({ children }: { children: ReactNode }) {
  return <div className="flex items-center gap-1.5 px-[9px] pt-3.5 pb-1 seclabel">{children}</div>;
}

export function Sidebar() {
  return (
    <aside className="flex w-[200px] flex-none flex-col gap-px overflow-y-auto border-r border-line bg-chrome px-2 py-2.5">
      <NavItem to="/" icon="inbox">
        Inbox
      </NavItem>
      <NavItem to="/agents" icon="agents">
        Agents
      </NavItem>
      <NavItem to="/projects" icon="folder">
        Projects
      </NavItem>
      <div className="mt-auto">
        <NavItem to="/settings" icon="sliders">
          Settings
        </NavItem>
      </div>
    </aside>
  );
}
