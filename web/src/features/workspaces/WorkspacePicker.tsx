// The workspace of a new folder: every folder belongs to exactly one workspace. Chips pick an existing one;
// "New workspace" creates one in place (the first folder of a fresh install starts here).

import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import type { Workspace } from "@/gen/Workspace";
import { Icon } from "@/components/Icon";
import { toastError } from "@/components/Toast";
import { Choice, Choices } from "@/components/ui";
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
    <div className="flex min-w-0 flex-wrap items-center gap-1.5">
      <span className="text-[12px] text-ink-3">{label}</span>
      {workspaces.length > 0 && <Choices label={label}>{workspaces.map((w) => <Choice key={w.id} active={value === w.id} onClick={() => onChange(w.id)}>{w.name}</Choice>)}</Choices>}
      {form ? (
        <form className="flex items-center gap-1.5" onSubmit={(e) => { e.preventDefault(); void create(); }}>
          <input autoFocus={workspaces.length > 0} className="field-input w-[180px]" maxLength={40} aria-label="New workspace name" placeholder={workspaces.length ? "New workspace name" : "Name a workspace, e.g. Work"} value={name} onChange={(e) => setName(e.target.value)} />
          <button type="submit" className="btn btn-sm" disabled={!name.trim() || busy}>Create</button>
          {workspaces.length > 0 && <button type="button" className="btn btn-sm btn-ghost" onClick={() => setTyping(false)}>Cancel</button>}
        </form>
      ) : (
        <button type="button" className="btn btn-sm btn-ghost" onClick={() => setTyping(true)}><Icon name="plus" size={12} />New workspace</button>
      )}
    </div>
  );
}

/** Setup's Workspaces step: the workspaces that exist and a form to create one more. There is no default workspace:
 * the owner names at least one before adding or cloning a folder. */
export function WorkspaceCreator() {
  const qc = useQueryClient();
  const workspaces = useWorkspaces().data ?? [];
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const create = async () => {
    const n = name.trim();
    if (!n) return;
    setBusy(true);
    try {
      await api.post<Workspace>("/api/workspaces", { name: n });
      setName("");
      await qc.invalidateQueries({ queryKey: ["workspaces"] });
      void qc.invalidateQueries({ queryKey: ["setup-checklist"] });
      void qc.invalidateQueries({ queryKey: ["setup-summary"] });
    } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  return (
    <form className="flex flex-wrap items-center gap-1.5" onSubmit={(e) => { e.preventDefault(); void create(); }}>
      <input className="field-input w-[220px]" maxLength={40} aria-label="Workspace name" placeholder={workspaces.length ? "Another workspace (optional)" : "Workspace name, e.g. Work"} value={name} onChange={(e) => setName(e.target.value)} />
      <button type="submit" className={`btn btn-sm${workspaces.length ? "" : " btn-primary"}`} disabled={!name.trim() || busy}><Icon name="plus" size={12} />Create workspace</button>
    </form>
  );
}

/** The default workspace of a new folder: the window's. */
export function defaultWorkspace(current: Workspace, all: Workspace[]): string | null {
  return all.some((w) => w.id === current.id) ? current.id : all[0]?.id ?? null;
}
