import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Icon } from "@/components/Icon";
import { toast, toastError } from "@/components/Toast";
import type { Project } from "@/gen/Project";
import { api } from "@/lib/api";
import { useConfig } from "@/lib/queries";

/** Keep as folder: moves a temporary folder into the folders root under the name you give it. */
export function KeepFolder({ p, disabled, title }: { p: Project; disabled?: boolean; title?: string }) {
  const qc = useQueryClient();
  const root = (useConfig().data?.config.projects_root ?? "~/projects").replace(/\/$/, "") + "/";
  const [name, setName] = useState(p.name.replace(/^tmp-\d{4}-/, ""));
  const keep = useMutation({
    mutationFn: () => api.post<Project>(`/api/projects/${p.id}/promote`, { name: name.trim() }),
    onSuccess: (np) => {
      toast(`Kept as ${np.path}`);
      void qc.invalidateQueries({ queryKey: ["projects"] });
      void qc.invalidateQueries({ queryKey: ["sessions"] });
    },
    onError: toastError,
  });
  return (
    <div className="flex flex-wrap items-center gap-2">
      <div className="flex h-[30px] min-w-0 flex-1 basis-48 items-center overflow-hidden rounded-[7px] border border-line">
        <span className="flex h-full max-w-[45%] min-w-0 items-center border-r border-line bg-sunk px-2 font-mono text-xs text-ink-3" title={root}><span className="truncate [direction:rtl]">{root}</span></span>
        <input className="h-full min-w-0 flex-1 px-2 font-mono text-xs outline-none" aria-label="Folder name" value={name} onChange={(e) => setName(e.target.value)} />
      </div>
      <button className="btn" disabled={disabled || !name.trim() || keep.isPending} title={title} onClick={() => keep.mutate()}>
        <Icon name="up" size={14} />
        Keep as folder
      </button>
    </div>
  );
}
