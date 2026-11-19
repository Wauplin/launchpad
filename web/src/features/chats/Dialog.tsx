// Small dialogs of the Chats pages: centered on the laptop, a sheet on the phone; Rename of a chat (its thread).

import { useQueryClient } from "@tanstack/react-query";
import { useState, type ReactNode } from "react";
import { toastError } from "@/components/Toast";
import { IconButton } from "@/components/ui";
import { runAction } from "@/lib/actions";
import { usePhone } from "@/lib/phone";
import { Sheet } from "@/mobile/Sheet";

/** A small dialog: centered on the laptop, a sheet on the phone. `wide` for editors. */
export function Dialog({ title, onClose, wide, children }: { title: string; onClose: () => void; wide?: boolean; children: ReactNode }) {
  const phone = usePhone();
  if (phone) return <Sheet title={title} full={wide} onClose={onClose}><div className="grid gap-3 p-3">{children}</div></Sheet>;
  return <div className="fixed inset-0 z-50 grid place-items-center bg-black/35 p-6" onClick={onClose} onKeyDown={(e) => { if (e.key === "Escape") onClose(); }}>
    <div role="dialog" aria-modal="true" aria-label={title} className={`flex max-h-[calc(86vh/var(--lp-zoom,1))] ${wide ? "w-[min(900px,calc(92vw/var(--lp-zoom,1)))]" : "w-[min(480px,calc(92vw/var(--lp-zoom,1)))]"} flex-col overflow-hidden rounded-card border border-line bg-paper shadow-[0_20px_60px_-20px_rgba(23,32,43,.6)]`} onClick={(e) => e.stopPropagation()}>
      <div className="flex items-center gap-2 border-b border-line-2 px-4 py-2.5">
        <b className="min-w-0 flex-1 truncate text-[15px]">{title}</b>
        <IconButton icon="x" label="Close" onClick={onClose} />
      </div>
      <div className="grid min-h-0 flex-1 gap-3 overflow-y-auto p-4">{children}</div>
    </div>
  </div>;
}

/** Rename a chat: a new title for its thread (one line). */
export function RenameThreadDialog({ bot, thread, title: initial, onClose }: { bot: string; thread: string; title: string; onClose: () => void }) {
  const qc = useQueryClient();
  const [title, setTitle] = useState(initial);
  const save = async () => {
    try { await runAction("bot.thread-rename", { id: bot, thread }, { title: title.trim() }); await qc.invalidateQueries({ queryKey: ["bot-threads"] }); onClose(); } catch (e) { toastError(e); }
  };
  return <Dialog title="Rename this chat" onClose={onClose}>
    <label className="grid gap-1 text-[12px] font-semibold text-ink-2">Title
      <input className="field-input font-normal" maxLength={80} value={title} autoFocus onChange={(e) => setTitle(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter" && title.trim()) void save(); }} />
    </label>
    <div className="flex justify-end gap-2">
      <button className="btn btn-sm btn-ghost" onClick={onClose}>Cancel</button>
      <button className="btn btn-sm btn-primary" disabled={!title.trim()} onClick={() => void save()}>Rename</button>
    </div>
  </Dialog>;
}
