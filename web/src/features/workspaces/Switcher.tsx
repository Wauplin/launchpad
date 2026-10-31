// Workspaces: the top bar switcher. Up to three workspaces are visible buttons;
// with more, one menu keeps the top bar on one row. The sidebar's workspace menu creates, renames and deletes them.

import { Icon } from "@/components/Icon";
import { MenuItem, Popover } from "@/components/Popover";
import { useProjects } from "@/lib/queries";
import { newWindowWord, openWindow, useWorkspaces } from "@/lib/workspace";
import { useWorkspace } from "@/stores/workspace";

export const EVERYTHING = "Everything";

function WorkspaceButton({ label, detail, active, onSwitch }: { label: string; detail: string; active: boolean; onSwitch: () => void }) {
  return (
    <button className="lp-chip max-w-[180px]" onClick={onSwitch} aria-pressed={active} title={`${label} · ${detail} · Show in this window`}>
      <span className="truncate">{label}</span>
    </button>
  );
}

export function WorkspaceSwitcher() {
  const { current, switchTo } = useWorkspace();
  const workspaces = useWorkspaces().data ?? [];
  const projects = useProjects().data?.length ?? 0;
  const active = workspaces.find((w) => w.id === current);
  const count = (n: number) => `${n} folder${n === 1 ? "" : "s"}`;
  const windowMenu = (close: () => void) => (
    <>
      <div className="seclabel px-2 pt-1 pb-0.5">Open in a new {newWindowWord}</div>
      <MenuItem label={EVERYTHING} detail={count(projects)} icon={<Icon name="window" size={13} />} onClick={() => { close(); void openWindow(null); }} />
      {workspaces.map((w) => <MenuItem key={w.id} label={w.name} detail={count(w.project_ids.length)} icon={<Icon name="window" size={13} />} onClick={() => { close(); void openWindow(w.id); }} />)}
    </>
  );
  if (workspaces.length > 3)
    return (
      <Popover width={280} trigger={({ open, toggle }) => (
        <button className="btn max-w-[220px]" data-tour="topbar.workspaces" aria-expanded={open} title="Workspace of this window" onClick={toggle}>
          <span className="truncate font-semibold">{active?.name ?? EVERYTHING}</span>
          <Icon name="chev" size={12} />
        </button>
      )}>
        {(close) => (
          <>
            <div className="seclabel px-2 pt-1 pb-0.5">Show in this {newWindowWord}</div>
            <MenuItem label={EVERYTHING} detail={`No filter · ${count(projects)}`} active={!active} onClick={() => { close(); switchTo(null); }} />
            {workspaces.map((w) => <MenuItem key={w.id} label={w.name} detail={count(w.project_ids.length)} active={w.id === active?.id} onClick={() => { close(); switchTo(w.id); }} />)}
            <div className="my-1 border-t border-line-2" />
            {windowMenu(close)}
          </>
        )}
      </Popover>
    );
  return (
    <div className="flex min-w-0 items-center gap-1.5" role="group" aria-label="Workspace of this window" data-tour="topbar.workspaces">
      <WorkspaceButton label={EVERYTHING} detail={`No filter · ${count(projects)}`} active={!active} onSwitch={() => switchTo(null)} />
      {workspaces.map((w) => (
        <WorkspaceButton key={w.id} label={w.name} detail={count(w.project_ids.length)} active={w.id === active?.id} onSwitch={() => switchTo(w.id)} />
      ))}
      <Popover trigger={({ open, toggle }) => (
        <button className="iconbtn" aria-expanded={open} aria-label={`Open a workspace in a new ${newWindowWord}`} title={`Open a workspace in a new ${newWindowWord}`} onClick={toggle}><Icon name="window" size={14} /></button>
      )}>
        {windowMenu}
      </Popover>
    </div>
  );
}
