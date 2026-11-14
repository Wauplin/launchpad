// Add folder: register a folder of Home. Clone a GitHub repository (Home mirrors it, runners clone it on first use)
// or create a new plain folder (files without Git, stored by Home). A temporary folder becomes a folder with Keep as
// folder. Every folder belongs to exactly one workspace, chosen here (the window's by default).

import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import type { Project } from "@/gen/Project";
import { Icon } from "@/components/Icon";
import { toast, toastError } from "@/components/Toast";
import { Tabs } from "@/components/ui";
import { api } from "@/lib/api";
import { usePhone } from "@/lib/phone";
import { useScope, useWorkspaces } from "@/lib/workspace";
import { WorkspacePicker, defaultWorkspace } from "@/features/workspaces/WorkspacePicker";
import { Sheet } from "@/mobile/Sheet";
import { CloneFromGitHub } from "./CloneFromGitHub";
import { useAddFolder } from "./addFolderStore";

/** Mounted once per shell; opened by `useAddFolder.getState().show()`. */
export function AddFolderDialog() {
  const open = useAddFolder((s) => s.open);
  const close = useAddFolder((s) => s.close);
  const phone = usePhone();
  if (!open) return null;
  if (phone) return <Sheet title="Add folder" full onClose={close}><Body onClose={close} /></Sheet>;
  return (
    <div className="fixed inset-0 z-50 grid place-items-center bg-black/35 p-6" onClick={close}>
      <div role="dialog" aria-label="Add folder" className="flex max-h-[calc(86vh/var(--lp-zoom,1))] w-[640px] max-w-full flex-col overflow-hidden rounded-card border border-line bg-paper shadow-[0_20px_60px_-20px_rgba(23,32,43,.6)]" onClick={(e) => e.stopPropagation()}>
        <div className="flex items-center gap-2 border-b border-line-2 px-4 py-2.5">
          <b className="flex-1 text-[15px]">Add folder</b>
          <button className="iconbtn" aria-label="Close" title="Close" onClick={close}><Icon name="x" size={14} /></button>
        </div>
        <Body onClose={close} />
      </div>
    </div>
  );
}

function Body({ onClose }: { onClose: () => void }) {
  const workspaces = useWorkspaces().data ?? [];
  const current = useScope().workspace;
  const [tab, setTab] = useState<"clone" | "plain">("clone");
  const [chosenId, setChosenId] = useState<string | null>(null);
  const chosen = workspaces.find((w) => w.id === (chosenId ?? defaultWorkspace(current, workspaces))) ?? null;
  return (
    <div className="grid min-h-0 flex-1 content-start gap-3 overflow-y-auto p-4">
      <Tabs label="How to add" className="min-h-[33px]" value={tab} onChange={setTab} items={[{ id: "clone" as const, label: "Clone from GitHub", icon: "branch" as const }, { id: "plain" as const, label: "New plain folder", icon: "plus" as const }]} />
      {tab === "plain" ? <NewPlainFolder workspaceId={chosen?.id ?? null} onDone={onClose} /> : <CloneFromGitHub workspace={chosen} onDone={onClose} />}
      <WorkspacePicker label="Add to" value={chosen?.id ?? null} onChange={setChosenId} />
    </div>
  );
}

/** A plain folder: files without Git, stored by Home with previous versions and trash. */
function NewPlainFolder({ workspaceId, onDone }: { workspaceId: string | null; onDone: () => void }) {
  const qc = useQueryClient();
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const create = async () => {
    setBusy(true);
    try {
      await api.post<Project>("/api/projects/plain", { name: name.trim(), workspace_id: workspaceId });
      await qc.invalidateQueries({ queryKey: ["projects"] });
      toast(`${name.trim()} created`);
      onDone();
    } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  return <section className="grid gap-1.5">
    <span className="text-[12px] text-ink-3">For projects without Git. Home stores the files and keeps previous versions and deleted files for 30 days; one agent works in it at a time, in a synced copy.</span>
    <div className="flex gap-1.5">
      <input className="field-input min-w-0 flex-1" aria-label="Folder name" placeholder="Folder name" value={name} onChange={(e) => setName(e.target.value)} />
      <button className="btn btn-sm btn-primary" disabled={busy || !name.trim() || !workspaceId} onClick={() => void create()}>Create</button>
    </div>
  </section>;
}
