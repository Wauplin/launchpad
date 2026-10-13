import type { FileAttachment } from "@/gen/FileAttachment";
import type { ClipboardEvent } from "react";

declare global {
  interface Window {
    /** Linux desktop fallback for WebKitGTK's empty screenshot paste events. */
    __launchpadReadClipboardImage?: () => Promise<string | null>;
  }
}

/** Leave text paste native, and read screenshots only in response to a paste gesture. */
export function pastedImages(event: ClipboardEvent<HTMLTextAreaElement>): File[] | Promise<File[]> | null {
  const files = clipboardImages(event.clipboardData);
  if (files.length) {
    event.preventDefault();
    return files;
  }
  // Text/HTML/URI clipboard contents must retain the textarea's normal paste behavior.
  if (event.clipboardData.types.some((type) => type.startsWith("text/"))
    || ["text/plain", "text/html", "text/uri-list"].some((type) => event.clipboardData.getData(type))
    || !window.__launchpadReadClipboardImage) return null;
  // There is no DOM image to suppress here. Keep the native default available even
  // when WebKit omits types for text that it can still insert into the textarea.
  return window.__launchpadReadClipboardImage().then((data) => {
    if (!data) return [];
    const bytes = Uint8Array.from(atob(data.split(",")[1]), (c) => c.charCodeAt(0));
    return [new File([bytes], "Pasted image.png", { type: "image/png" })];
  });
}

/** Screenshots can appear in either clipboard collection depending on the webview. */
export function clipboardImages(data: DataTransfer): File[] {
  const files = Array.from(data.files).filter((file) => file.type.startsWith("image/"));
  if (files.length) return files;
  return Array.from(data.items).filter((item) => item.kind === "file" && item.type.startsWith("image/")).map((item) => item.getAsFile()).filter((file): file is File => !!file);
}

export async function readAttachments(files: File[], current: FileAttachment[], imagesOnly = false): Promise<FileAttachment[]> {
  if (current.length + files.length > 8) throw new Error("Attach at most 8 files.");
  const added = await Promise.all(files.map(async (file) => {
    if ((imagesOnly || file.type.startsWith("image/")) && !["image/png", "image/jpeg", "image/webp", "image/gif"].includes(file.type)) throw new Error("Use PNG, JPEG, WebP or GIF images.");
    if (file.size > 5 * 1024 * 1024) throw new Error("Each attachment must be 5 MB or smaller.");
    if (file.type.startsWith("image/") && !file.size) throw new Error("The image is empty.");
    const data_url = await new Promise<string>((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve(String(reader.result));
      reader.onerror = () => reject(new Error("Could not read the attachment."));
      reader.onabort = () => reject(new Error("Reading the attachment was cancelled."));
      reader.readAsDataURL(file);
    });
    return { name: file.name || "Pasted image.png", data_url };
  }));
  if ([...current, ...added].reduce((n, file) => n + file.data_url.length, 0) > 28_000_000) throw new Error("Attachments must total 20 MB or less.");
  return added;
}
