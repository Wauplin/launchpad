// Where an agent, project or terminal runs: a small host chip, colored by the connection state. Nothing for
// this computer unless `local` is set, so local-only setups look unchanged.

import { hostOf, linkLabel, useHost } from "@/lib/hosts";

export function HostLabel({ path, host, local = false, className = "" }: { path?: string | null; host?: string | null; local?: boolean; className?: string }) {
  const id = host ?? hostOf(path);
  const h = useHost(id);
  if (!id) return local ? <span className={`inline-flex flex-none items-center rounded border border-line px-1 text-[11px] leading-[16px] text-ink-3 ${className}`}>local</span> : null;
  const state = h?.state ?? "unreachable";
  const dot = state === "connected" ? "bg-gh-open" : state === "connecting" ? "bg-attn" : "bg-gh-closed";
  return (
    <span
      className={`inline-flex flex-none items-center gap-1 rounded border border-line px-1 text-[11px] leading-[16px] font-semibold text-ink-2 ${className}`}
      title={`Runs on ${id} · ${linkLabel[state]}${h?.error ? `: ${h.error}` : ""}`}
    >
      <i className={`inline-block size-1.5 rounded-full ${dot}`} aria-hidden />
      {id}
    </span>
  );
}
