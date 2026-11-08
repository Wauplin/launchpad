import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Icon } from "@/components/Icon";
import { toast, toastError } from "@/components/Toast";
import type { Project } from "@/gen/Project";
import { api } from "@/lib/api";

/** Keep as folder: a temporary folder becomes a folder stored by Home, under the name you give it, so it runs on any
 * runner. Files become a plain folder; a Git repository becomes a Git folder. */
export function KeepFolder({ p, disabled, title }: { p: Project; disabled?: boolean; title?: string }) {
  const qc = useQueryClient();
  const [name, setName] = useState(p.name.replace(/^tmp-\d{4}-/, ""));
  const keep = useMutation({
    mutationFn: () => api.post<Project>(`/api/projects/${p.id}/promote`, { name: name.trim() }),
    onSuccess: (np) => {
      toast(`Kept as the folder ${np.name}, stored on Home`);
      void qc.invalidateQueries({ queryKey: ["projects"] });
      void qc.invalidateQueries({ queryKey: ["sessions"] });
    },
    onError: toastError,
  });
  return (
    <div className="flex flex-wrap items-center gap-2">
      <input className="field-input min-w-0 flex-1 basis-48 font-mono text-[12px]" aria-label="Folder name" value={name} onChange={(e) => setName(e.target.value)} />
      <button className="btn" disabled={disabled || !name.trim() || keep.isPending} title={title} onClick={() => keep.mutate()}>
        <Icon name="up" size={14} />
        Keep as folder
      </button>
    </div>
  );
}
