// Launchpad references added to a launch or a message (a pasted link or ID, Add context → Chat… or Channel…), as
// removable chips. The agent reads each with `launchpad ref read <id>` (Home names the command in its prompt).

import type { LpRef } from "@/gen/LpRef";
import { Icon } from "@/components/Icon";
import { describeRef, REF_ICON, refId } from "@/lib/refs";

export function RefChips({ refs = [], onRemove, disabled }: { refs?: LpRef[]; onRemove: (id: string) => void; disabled?: boolean }) {
  return refs.map((r) => {
    const id = refId(r);
    const label = describeRef(r);
    return <span key={id} title={`${label}: the agent reads it with launchpad ref read ${id}`} className="inline-flex h-7 min-w-0 max-w-full items-center gap-1 rounded-control border border-line bg-chrome pl-2 text-[12px] text-ink-2">
      <Icon name={REF_ICON[r.kind]} size={12} className="shrink-0" />
      <span className="truncate">{label}</span>
      <button className="iconbtn shrink-0" aria-label={`Remove ${label}`} title="Remove" disabled={disabled} onClick={() => onRemove(id)}><Icon name="x" size={12} /></button>
    </span>;
  });
}

/** Adds a reference once (by its short ID). */
export const addRef = (list: LpRef[] | undefined, r: LpRef): LpRef[] => (list ?? []).some((x) => refId(x) === refId(r)) ? list ?? [] : [...(list ?? []), r];
