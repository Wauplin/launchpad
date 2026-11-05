// The workspace rail (laptop): one square per workspace at the left edge, as in Slack. A window shows one workspace;
// the others show what waits in them as a count, never their content. A click shows a workspace in this window; its
// menu in the sidebar opens it in a new window instead.

import { memo } from "react";
import type { Workspace } from "@/gen/Workspace";
import { Icon } from "@/components/Icon";
import { Popover } from "@/components/Popover";
import { useWorkspaces } from "@/lib/workspace";
import { useWorkspace } from "@/stores/workspace";
import { describeCount, useWorkspaceCounts, workspaceInitial, type WorkspaceCount } from "./counts";
import { NewWorkspaceForm } from "./WorkspaceMenu";

const Square = memo(function Square({ w, active, count }: { w: Workspace; active: boolean; count: WorkspaceCount | undefined }) {
  const others = !active && !!count?.total;
  return (
    <button
      className={`relative grid size-[32px] flex-none place-items-center rounded-[8px] text-[13px] font-semibold ${active ? "bg-teal text-white shadow-[0_0_0_2px_var(--color-paper),0_0_0_4px_var(--color-teal)]" : "bg-sunk text-ink-2 hover:bg-select hover:text-ink"}`}
      aria-pressed={active}
      aria-label={w.name}
      title={active ? `${w.name} · shown in this window` : `${w.name} · ${describeCount(count)}. Click to show it here.`}
      onClick={() => { if (!active) useWorkspace.getState().switchTo(w.id); }}
    >
      {workspaceInitial(w.name)}
      {others && <span className="absolute -top-1.5 -right-1.5 min-w-[17px] rounded-full border-2 border-chrome bg-attn px-[3px] text-center text-[11px] leading-[13px] font-bold text-white tabular-nums">{count!.total}</span>}
    </button>
  );
});

export function WorkspaceRail() {
  const current = useWorkspace((s) => s.current);
  const workspaces = useWorkspaces().data ?? [];
  const counts = useWorkspaceCounts();
  return (
    <nav aria-label="Workspaces" data-tour="rail.workspaces" className="flex w-[46px] flex-none flex-col items-center gap-2.5 overflow-y-auto border-r border-line bg-chrome py-2.5">
      {workspaces.map((w) => <Square key={w.id} w={w} active={w.id === current} count={counts[w.id]} />)}
      <Popover width={260} trigger={({ open, toggle }) => (
        <button className="grid size-[32px] flex-none place-items-center rounded-[8px] border border-dashed border-line text-ink-3 hover:bg-sunk hover:text-ink" aria-expanded={open} aria-label="New workspace" title="New workspace" onClick={toggle}><Icon name="plus" size={14} /></button>
      )}>
        {(close) => <NewWorkspaceForm onDone={close} onCancel={close} />}
      </Popover>
    </nav>
  );
}
