// Download and image links (`daemonAsset`) in demo mode: data URLs built from the world, never a daemon request.
// `/api/code/download` and `/api/code/raw` of the fake files return their real content, `/api/uploads/<hash>` the
// upload's data URL, a channel message's file a short log; anything else a placeholder text.

import { openSource } from "./routes/code";
import { world } from "./world";

const placeholder = (path: string) => `data:text/plain;charset=utf-8,${encodeURIComponent(`Demo: ${path.split("?")[0]} is not available in the demo.`)}`;

export function demoAsset(path: string): string {
  const url = new URL(path, "http://demo.invalid");
  const upload = /^\/api\/uploads\/([0-9a-f]{64})$/.exec(url.pathname);
  if (upload) return world.uploads[upload[1]]?.data_url ?? placeholder(path);
  // A channel message's file (the owner's download, `owner_file` on Home): the demo's files are short logs.
  const channelFile = /^\/api\/channels\/([^/]+)\/messages\/(\d+)\/files\/(\d+)$/.exec(url.pathname);
  if (channelFile) {
    const m = world.channelMessages.find((x) => x.channel_id === decodeURIComponent(channelFile[1]) && x.id === Number(channelFile[2]));
    if (!m?.files[Number(channelFile[3])]) return placeholder(path);
    const log = "run 1182 · checkout-e2e\n✗ cart keeps items after reload (timeout 5000 ms)\n  retried: passed\nflaky: the test waited for a network idle that never came\n";
    return `data:application/octet-stream;charset=utf-8,${encodeURIComponent(log)}`;
  }
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
