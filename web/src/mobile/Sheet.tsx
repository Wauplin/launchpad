// Bottom sheet for the phone shell: slides up over the page, closes with its button or the backdrop.

import type { ReactNode } from "react";
import { Icon } from "@/components/Icon";

export function Sheet({ title, onClose, children, full }: { title: string; onClose: () => void; children: ReactNode; full?: boolean }) {
  return (
    <div className="fixed inset-0 z-50 flex flex-col justify-end bg-black/35" onClick={onClose}>
      <div
        role="dialog"
        aria-label={title}
        className={`flex flex-col rounded-t-2xl border-t border-line bg-paper pb-[env(safe-area-inset-bottom)] shadow-[0_-10px_30px_rgba(0,0,0,.25)] ${full ? "h-[92dvh]" : "max-h-[85dvh]"}`}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex flex-none items-center gap-2 border-b border-line-2 py-1 pr-1 pl-4">
          <b className="flex-1 truncate text-[15px]">{title}</b>
          <button className="iconbtn" aria-label="Close" title="Close" onClick={onClose}><Icon name="x" size={16} /></button>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto">{children}</div>
      </div>
    </div>
  );
}
