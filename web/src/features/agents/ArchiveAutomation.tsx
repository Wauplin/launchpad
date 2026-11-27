// Archive automation threads: a preview of the finished automation threads of the workspace (routine runs and the
// threads their handoffs, channels, DMs, hooks and other agents started), then one confirmed archive of exactly
// those (`session.archive-done`, which checks each one again on Home).

import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { refreshThreads } from "@/lib/bots";
import { runAction } from "@/lib/actions";
import { ago } from "@/lib/format";
import { Choice, Choices, ConfirmButton, Empty, ErrorState, Loading, StatusBadge } from "@/components/ui";
import { toast, toastError } from "@/components/Toast";
import { Dialog } from "@/features/chats/Dialog";
import type { ArchivePreview } from "@/gen/ArchivePreview";
import type { ArchiveOutcome } from "@/gen/ArchiveOutcome";

const AGES = [["", "Any age"], ["1h", "1 h"], ["1d", "1 d"], ["3d", "3 d"], ["7d", "7 d"]] as const;

/** The dialog; `bot` narrows it to the conversations of one bot (its page). */
export function ArchiveAutomationDialog({ workspace, bot, onClose }: { workspace: string; bot?: { id: string; name: string }; onClose: () => void }) {
  const qc = useQueryClient();
  const [olderThan, setOlderThan] = useState("");
  const params = new URLSearchParams({ workspace, ...(bot ? { bot: bot.id } : {}), ...(olderThan ? { older_than: olderThan } : {}) });
  const preview = useQuery({ queryKey: ["archive-done", workspace, bot?.id ?? "", olderThan], queryFn: () => api.get<ArchivePreview>(`/api/context/archive-done?${params}`), staleTime: 0 });
  const items = preview.data?.items ?? [];
  const kept = preview.data?.kept ?? [];

  const apply = async () => {
    try {
      const out = await runAction<ArchiveOutcome>("session.archive-done", { workspace }, { session_ids: items.map((i) => i.session_id), bot: bot?.id ?? null, older_than: olderThan || null });
      await Promise.all([qc.invalidateQueries({ queryKey: ["sessions"] }), refreshThreads(qc)]);
      const left = out.kept.length + out.failed.length;
      if (out.failed.length) toastError(new Error(`${out.title}. ${out.failed.length} could not be confirmed: check them in Agents. ${out.failed[0].title}: ${out.failed[0].reason}`));
      else toast(left ? `${out.title}. ${left} changed since the preview and stayed.` : `${out.title}. Unarchive brings each one back.`);
      onClose();
    } catch (e) { toastError(e); }
  };

  return <Dialog title={bot ? `Archive ${bot.name}'s automation threads` : "Archive automation threads"} wide onClose={onClose}>
    <p className="text-[13px] text-ink-2">Finished routine runs and the threads that routines, channels, DMs, hooks or other agents started. Threads you started, and anything that asks you, waits for you, sleeps, works or failed, stay. Traces are kept, and Unarchive brings each one back.</p>
    <Choices label="Done for at least">
      {AGES.map(([v, label]) => <Choice key={v} active={olderThan === v} onClick={() => setOlderThan(v)}>{label}</Choice>)}
    </Choices>
    {preview.isLoading ? <Loading what="the threads" /> : preview.error ? <ErrorState error={preview.error} onRetry={() => void preview.refetch()} /> : <>
      {items.length === 0 ? <Empty icon="archive">No finished automation thread to archive.</Empty> : <div className="grid gap-1">
        <b className="text-[13px]">{items.length} to archive</b>
        <ul className="grid rounded-card border border-line">
          {items.map((i) => <li key={i.session_id} className="flex min-w-0 items-center gap-2 border-b border-line-2 px-3 py-1.5 text-[12px] last:border-b-0">
            {i.bot_name && <span className="shrink-0 text-ink-3">{i.bot_name}</span>}
            <span className="min-w-0 flex-1 truncate" title={i.title}>{i.title}</span>
            <span className="shrink-0 text-ink-3">{i.origin}</span>
            <StatusBadge tone="done" label="Done" unseen={i.state === "unread"} />
            <span className="w-16 shrink-0 text-right text-ink-3">{ago(i.done_at)}</span>
          </li>)}
        </ul>
      </div>}
      {kept.length > 0 && <details className="text-[12px] text-ink-2">
        <summary className="cursor-pointer">{kept.length} automation thread{kept.length === 1 ? "" : "s"} stay</summary>
        <ul className="mt-1 grid gap-0.5">
          {kept.map((k) => <li key={k.session_id} className="min-w-0 truncate" title={`${k.title}: ${k.reason}`}>{k.bot_name ? `${k.bot_name} · ` : ""}{k.title}: <span className="text-ink-3">{k.reason}</span></li>)}
        </ul>
      </details>}
      {items.length > 0 && <div className="flex justify-end">
        <ConfirmButton small={false} icon="archive" consequence={`Archive these ${items.length}?`} confirmLabel="Archive" onConfirm={apply}>{`Archive ${items.length}`}</ConfirmButton>
      </div>}
    </>}
  </Dialog>;
}
