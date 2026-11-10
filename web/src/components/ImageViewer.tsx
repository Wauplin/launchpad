// Full-screen image viewer for images in messages and attachments: the image fit to the window, pinch zoom left to
// the browser, a bar with the file name, Download and ✕. Closes on ✕, Escape, a tap on the backdrop and Android back
// (its own history entry, `useBackClose`). The same viewer on the laptop and the phone.

import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { Icon } from "@/components/Icon";
import { IconButton } from "@/components/ui";
import { useBackClose } from "@/mobile/back";

export function ImageViewer({ src, name, onClose }: { src: string; name: string; onClose: () => void }) {
  useBackClose(true, onClose);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") { e.stopPropagation(); onClose(); } };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onClose]);
  return createPortal(
    <div role="dialog" aria-modal="true" aria-label={name} className="fixed inset-0 z-[90] flex flex-col bg-black/85" onClick={onClose}>
      <div className="flex flex-none items-center gap-1 border-b border-line bg-paper py-1 pr-1 pl-4 pt-[max(4px,env(safe-area-inset-top))]" onClick={(e) => e.stopPropagation()}>
        <b className="min-w-0 flex-1 truncate text-[13px]" title={name}>{name}</b>
        <a className="iconbtn" href={src} download={name} title={`Download ${name}`} aria-label={`Download ${name}`}><Icon name="download" size={14} /></a>
        <IconButton icon="x" label="Close" onClick={onClose} />
      </div>
      <div className="flex min-h-0 flex-1 items-center justify-center p-2 pb-[max(8px,env(safe-area-inset-bottom))]">
        <img src={src} alt={name} className="max-h-full max-w-full object-contain [touch-action:pinch-zoom]" onClick={(e) => e.stopPropagation()} />
      </div>
    </div>,
    document.body,
  );
}

/** An image thumbnail that opens the viewer on a tap. */
export function ImageThumb({ src, name, className }: { src: string; name: string; className: string }) {
  const [open, setOpen] = useState(false);
  return <>
    <button type="button" className="max-w-full cursor-zoom-in" title={`Open ${name}`} aria-label={`Open ${name}`} onClick={() => setOpen(true)}>
      <img src={src} alt={name} className={className} />
    </button>
    {open && <ImageViewer src={src} name={name} onClose={() => setOpen(false)} />}
  </>;
}
