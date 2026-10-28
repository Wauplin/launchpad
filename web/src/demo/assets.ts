// Download and image links (`daemonAsset`) in demo mode: data URLs built from the world, never a daemon request.
// `/api/code/download` and `/api/code/raw` of the fake files return their real content; anything else a
// placeholder text.

import { openSource } from "./routes/code";

const placeholder = (path: string) => `data:text/plain;charset=utf-8,${encodeURIComponent(`Demo: ${path.split("?")[0]} is not available in the demo.`)}`;

export function demoAsset(path: string): string {
  const url = new URL(path, "http://demo.invalid");
  if (url.pathname !== "/api/code/download" && url.pathname !== "/api/code/raw") return placeholder(path);
  const root = url.searchParams.get("root"), file = url.searchParams.get("path");
  if (!root || !file) return placeholder(path);
  try {
    const text = openSource(root, url.searchParams.get("rev")).files[file];
    if (text === undefined) return placeholder(path);
    const type = file.endsWith(".svg") ? "image/svg+xml" : url.pathname === "/api/code/download" ? "application/octet-stream" : "text/plain";
    return `data:${type};charset=utf-8,${encodeURIComponent(text)}`;
  } catch {
    return placeholder(path);
  }
}
