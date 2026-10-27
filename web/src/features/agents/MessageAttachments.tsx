// Attachments of a message before it is sent: the session message box and the first Ask Launchpad message.
// Files upload once (`readAttachments`); images reach the harness as native input, other files as local copies.

import { useRef, useState } from "react";
import type { FileAttachment } from "@/gen/FileAttachment";
import { Icon } from "@/components/Icon";
import { toastError } from "@/components/Toast";
import { attachmentSrc, isImageAttachment, readAttachments } from "@/lib/attachments";

export const ATTACH_LIMITS = "Up to 8 files, 5 MB each, 20 MB total.";

/** Pending attachments with one read at a time; `images` is false when the harness cannot take images. */
export function useMessageAttachments(images: boolean) {
  const [files, setFiles] = useState<FileAttachment[]>([]);
  const [reading, setReading] = useState(false);
  const readingRef = useRef(false);
  const current = useRef(files);
  current.current = files;
  const add = async (input: File[] | Promise<File[]>) => {
    if (readingRef.current) return;
    readingRef.current = true;
    setReading(true);
    try {
      const picked = await input;
      if (!picked.length) return;
      const added = await readAttachments(picked, current.current);
      if (!images && added.some((f) => isImageAttachment(f.data_url))) throw new Error("This harness does not support images. Attach other files only.");
      setFiles((list) => [...list, ...added]);
    } catch (e) { toastError(e); }
    finally { readingRef.current = false; setReading(false); }
  };
  return { files, reading, readingRef, add, remove: (i: number) => setFiles((list) => list.filter((_, j) => j !== i)), clear: () => setFiles([]) };
}

/** Thumbnails for images and name chips for other files, each with a remove button. */
export function AttachmentList({ files, disabled, onRemove }: { files: FileAttachment[]; disabled?: boolean; onRemove: (i: number) => void }) {
  if (!files.length) return null;
  return (
    <div className="mb-2 flex flex-wrap items-start gap-2">{files.map((file, i) => (
      <div key={i} className="relative max-w-full rounded-md border border-line bg-chrome p-1.5 pr-6">
        {isImageAttachment(file.data_url)
          ? <img src={attachmentSrc(file.data_url)} alt={file.name} className="mb-1 h-16 max-w-28 object-contain" />
          : <Icon name="file" size={14} className="mb-1 text-ink-3" />}
        <div className="max-w-48 truncate text-[12px]" title={file.name}>{file.name}</div>
        <button className="iconbtn absolute top-0 right-0" title={`Remove ${file.name}`} aria-label={`Remove ${file.name}`} disabled={disabled} onClick={() => onRemove(i)}><Icon name="x" size={12} /></button>
      </div>
    ))}</div>
  );
}
