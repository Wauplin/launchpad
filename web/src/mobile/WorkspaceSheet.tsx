// The phone's workspace sheet: your workspaces with what waits in each (never their content), and New workspace.
// Nothing else: the other places are in the More tab.

import { useState } from "react";
import { Icon } from "@/components/Icon";
import { describeCount, useWorkspaceCounts, workspaceInitial } from "@/features/workspaces/counts";
import { NewWorkspaceForm } from "@/features/workspaces/WorkspaceMenu";
import { useWorkspaces } from "@/lib/workspace";
import { useWorkspace } from "@/stores/workspace";
import { Sheet } from "./Sheet";

export function WorkspaceSheet({ onClose }: { onClose: () => void }) {
  const { current, switchTo } = useWorkspace();
  const workspaces = useWorkspaces().data ?? [];
  const counts = useWorkspaceCounts();
  const [creating, setCreating] = useState(false);
  const pick = (id: string) => { switchTo(id); onClose(); };
  return (
    <Sheet title="Workspaces" onClose={onClose}>
      <div data-tour="phone.workspaces" className="pb-2">
        {workspaces.map((w) => {
          const on = current === w.id;
          const n = on ? 0 : counts[w.id]?.total ?? 0;
          return (
            <button key={w.id} className={`flex min-h-12 w-full items-center gap-3 border-b border-line-2 px-4 text-left text-[15px] ${on ? "font-semibold text-teal-ink" : ""}`} aria-pressed={on} title={on ? undefined : describeCount(counts[w.id])} onClick={() => pick(w.id)}>
              <span className={`grid size-[28px] flex-none place-items-center rounded-[7px] text-[13px] font-semibold ${on ? "bg-teal text-white" : "bg-sunk text-ink-2"}`} aria-hidden>{workspaceInitial(w.name)}</span>
              <span className="flex-1 truncate">{w.name}</span>
              {n > 0 && <span className="min-w-[22px] rounded-full bg-attn-soft px-1.5 text-center text-[13px] leading-[22px] font-semibold text-attn tabular-nums" aria-label={describeCount(counts[w.id])}>{n}</span>}
              {on && <Icon name="check" size={16} />}
            </button>
          );
        })}
        {creating ? <NewWorkspaceForm onDone={onClose} onCancel={() => setCreating(false)} /> : (
          <button className="flex min-h-12 w-full items-center gap-3 border-b border-line-2 px-4 text-left text-[15px] text-teal-ink" onClick={() => setCreating(true)}>
            <span className="grid size-[28px] flex-none place-items-center rounded-[7px] border border-dashed border-line" aria-hidden><Icon name="plus" size={14} /></span>New workspace
          </button>
        )}
      </div>
    </Sheet>
  );
}
