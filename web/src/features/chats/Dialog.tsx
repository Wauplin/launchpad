// Small dialogs of the Chats pages: centered on the laptop, a sheet on the phone.

import type { ReactNode } from "react";
import { IconButton } from "@/components/ui";
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
