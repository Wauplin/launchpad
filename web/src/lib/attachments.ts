import { daemonUrl } from "./connection.ts";
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

const IMAGE_TYPES = ["image/png", "image/jpeg", "image/webp", "image/gif"];
/** Longest side of re-encoded photos: enough for an agent to read a screenshot, far smaller than a camera image. */
const MAX_SIDE = 2048;
/** Images in a supported format at most this large are sent as they are. */
const KEEP_BELOW = 1.5 * 1024 * 1024;

/** Re-encodes large or unsupported images (camera photos, HEIC where the browser can decode it) to a JPEG of at most 2048 px. */
async function normalizeImage(file: File): Promise<File> {
  const supported = IMAGE_TYPES.includes(file.type);
  if (supported && (file.type === "image/gif" || file.size <= KEEP_BELOW)) return file;
  let bitmap: ImageBitmap;
  try { bitmap = await createImageBitmap(file, { imageOrientation: "from-image" }); }
  catch {
    if (supported) return file;
    throw new Error(`${file.name || "This image"}: this browser cannot read the format. Use PNG, JPEG, WebP or GIF.`);
  }
  const scale = Math.min(1, MAX_SIDE / Math.max(bitmap.width, bitmap.height));
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(bitmap.width * scale));
  canvas.height = Math.max(1, Math.round(bitmap.height * scale));
  const ctx = canvas.getContext("2d");
  if (!ctx) return file;
  ctx.fillStyle = "#fff";
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  ctx.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
  bitmap.close();
  const blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, "image/jpeg", 0.88));
  if (!blob || (supported && blob.size >= file.size)) return file;
  const base = (file.name || "Photo").replace(/\.[^.]*$/, "");
  return new File([blob], `${base}.jpg`, { type: "image/jpeg" });
}

/** Uploads one file to the daemon once; drafts and launches then carry the short reference. */
async function upload(file: File): Promise<string> {
  const res = await fetch(daemonUrl("/api/uploads"), { credentials: "include", method: "POST", headers: { "Content-Type": file.type || "application/octet-stream" }, body: file });
  const data = await res.json().catch(() => null);
  if (res.status === 401) location.replace("/auth");
  if (!res.ok || typeof data?.data_url !== "string") throw new Error(data?.error ?? `Upload failed (${res.status}).`);
  return data.data_url;
}

/** Image source for an attachment: a data URL, or the daemon URL of an uploaded image. */
export function attachmentSrc(dataUrl: string): string {
  const m = /^upload:(image\/[a-z0-9.+-]+);([0-9a-f]{64})$/.exec(dataUrl);
  return m ? daemonUrl(`/api/uploads/${m[2]}?type=${encodeURIComponent(m[1])}`) : dataUrl.startsWith("/api/") ? daemonUrl(dataUrl) : dataUrl;
}

export const isImageAttachment = (dataUrl: string) => dataUrl.startsWith("data:image/") || dataUrl.startsWith("upload:image/");

export async function readAttachments(files: File[], current: FileAttachment[], imagesOnly = false): Promise<FileAttachment[]> {
  if (current.length + files.length > 8) throw new Error("Attach at most 8 files.");
  const prepared = await Promise.all(files.map(async (file) => {
    const looksImage = file.type.startsWith("image/") || (!file.type && /\.(heic|heif)$/i.test(file.name));
    if (imagesOnly && !looksImage) throw new Error("Attach images only.");
    const ready = looksImage ? await normalizeImage(file) : file;
    if (ready.size > 5 * 1024 * 1024) throw new Error("Each attachment must be 5 MB or smaller.");
    if (looksImage && !ready.size) throw new Error("The image is empty.");
    return ready;
  }));
  if (prepared.reduce((n, f) => n + f.size, 0) > 20 * 1024 * 1024) throw new Error("Attachments must total 20 MB or less.");
  return Promise.all(prepared.map(async (file) => ({ name: file.name || "Pasted image.png", data_url: await upload(file) })));
}
