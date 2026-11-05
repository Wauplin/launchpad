// The workspace of a new folder: every folder belongs to exactly one workspace. Chips pick an existing one;
// "New workspace" creates one in place (the first folder of a fresh install starts here).

import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import type { Workspace } from "@/gen/Workspace";
import { Icon } from "@/components/Icon";
import { toastError } from "@/components/Toast";
import { FilterChip, FilterChips } from "@/components/ui";
import { api } from "@/lib/api";
import { useWorkspaces } from "@/lib/workspace";

export function WorkspacePicker({ value, onChange, label = "Workspace" }: { value: string | null; onChange: (id: string) => void; label?: string }) {
  const qc = useQueryClient();
  const workspaces = useWorkspaces().data ?? [];
  const [typing, setTyping] = useState(false);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const create = async () => {
    const n = name.trim();
    if (!n) return;
    setBusy(true);
    try {
      const w = await api.post<Workspace>("/api/workspaces", { name: n });
      await qc.invalidateQueries({ queryKey: ["workspaces"] });
      onChange(w.id);
      setTyping(false);
      setName("");
    } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  const form = typing || workspaces.length === 0;
  return (
    <FilterChips label={label}>
      <span className="text-[12px] text-ink-3">{label}</span>
      {workspaces.map((w) => <FilterChip key={w.id} active={value === w.id} onClick={() => onChange(w.id)}>{value === w.id && <Icon name="check" size={11} />}{w.name}</FilterChip>)}
      {form ? (
        <form className="flex items-center gap-1.5" onSubmit={(e) => { e.preventDefault(); void create(); }}>
          <input autoFocus={workspaces.length > 0} className="field-input h-[26px] w-[160px] text-[12px]" maxLength={40} aria-label="New workspace name" placeholder={workspaces.length ? "New workspace name" : "Name a workspace, e.g. Personal"} value={name} onChange={(e) => setName(e.target.value)} />
          <button type="submit" className="btn btn-sm" disabled={!name.trim() || busy}>Create</button>
          {workspaces.length > 0 && <button type="button" className="btn btn-sm btn-ghost" onClick={() => setTyping(false)}>Cancel</button>}
        </form>
      ) : (
        <button type="button" className="lp-chip" onClick={() => setTyping(true)}><Icon name="plus" size={11} />New workspace</button>
      )}
    </FilterChips>
  );
}

/** The default workspace of a new folder: the window's. */
export function defaultWorkspace(current: Workspace, all: Workspace[]): string | null {
  return all.some((w) => w.id === current.id) ? current.id : all[0]?.id ?? null;
}
