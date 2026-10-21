import { Icon } from "@/components/Icon";
import { Popover } from "@/components/Popover";
import { ProjectPicker } from "@/features/launch/ProjectPicker";
import { useOpenTerminal } from "@/features/terminals/lib";
import { useScope } from "@/lib/workspace";

/** Pick a project in this workspace so the new terminal is visible in its dock. */
export function TerminalButton() {
  const scope = useScope();
  const openTerminal = useOpenTerminal();
  const projects = [...scope.projects].sort((a, b) => Number(b.pinned) - Number(a.pinned) || a.name.localeCompare(b.name));
  return <Popover
    align="right"
    width={300}
    label="Open a terminal"
    trigger={({ open, toggle }) => <button className="btn" title="Open a new terminal in a project" aria-haspopup="dialog" aria-expanded={open} onClick={toggle}>
      <Icon name="term" size={14} />
      Terminal
      <Icon name="chev" size={12} />
    </button>}
  >
    {(close) => <ProjectPicker
      projects={projects}
      hasProject={scope.hasProject}
      temporary={false}
      emptyMessage={scope.workspace ? "No projects in this workspace. Add some in Projects." : undefined}
      onPick={(project) => {
        if (!project) return;
        close();
        void openTerminal.shell(project.path);
      }}
    />}
  </Popover>;
}
