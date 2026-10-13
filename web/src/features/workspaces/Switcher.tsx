// Workspaces (ARCHITECTURE.md, 9.1.2): the top bar switcher. Kept apart from the Settings page so
// that page can load lazily.

import { Icon } from "@/components/Icon";
import { MenuItem, Popover } from "@/components/Popover";
import { useProjects } from "@/lib/queries";
import { openWindow, useWorkspaces } from "@/lib/workspace";
import { useWorkspace } from "@/stores/workspace";

function WorkspaceButton({ label, detail, active, onSwitch }: { label: string; detail: string; active: boolean; onSwitch: () => void }) {
  return (
    <button className={`min-w-0 rounded-md border px-2.5 py-1 text-[12.5px] ${active ? "border-teal/40 bg-teal-soft text-teal-ink" : "border-line bg-paper text-ink-2 hover:bg-select"}`} onClick={onSwitch} aria-pressed={active} title={`${label} · ${detail} · Show in this window`}>
      <span className={`block truncate ${active ? "font-semibold" : ""}`}>{label}</span>
    </button>
  );
}

export function WorkspaceSwitcher() {
  const { current, switchTo } = useWorkspace();
  const workspaces = useWorkspaces().data ?? [];
  const projects = useProjects().data?.length ?? 0;
  const active = workspaces.find((w) => w.id === current);
  const count = (n: number) => `${n} project${n === 1 ? "" : "s"}`;
  return (
    <div className="flex min-w-0 max-w-full flex-wrap items-center gap-1.5" role="group" aria-label="Workspace of this window">
      <WorkspaceButton label="All workspaces" detail={`No filter · ${count(projects)}`} active={!active} onSwitch={() => switchTo(null)} />
      {workspaces.map((w) => (
        <WorkspaceButton key={w.id} label={w.name} detail={count(w.project_ids.length)} active={w.id === active?.id} onSwitch={() => switchTo(w.id)} />
      ))}
      <Popover
        trigger={({ open, toggle }) => (
          <button className="btn btn-sm btn-ghost" aria-expanded={open} title="Open a workspace in a new window" onClick={toggle}>
            <Icon name="window" size={13} />
            Open in new window ▾
          </button>
        )}
      >
        {(close) => (
          <>
            <MenuItem label="All workspaces" detail={count(projects)} icon={<Icon name="window" size={13} />} onClick={() => { close(); void openWindow(null); }} />
            {workspaces.map((w) => <MenuItem key={w.id} label={w.name} detail={count(w.project_ids.length)} icon={<Icon name="window" size={13} />} onClick={() => { close(); void openWindow(w.id); }} />)}
          </>
        )}
      </Popover>
    </div>
  );
}
