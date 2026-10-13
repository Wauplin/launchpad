// Which daemon this window talks to, and whether the live connection works. Shown in browser tabs
// (the laptop status bar, the phone top bar); the desktop shell runs on the daemon's machine.

import { useEffect, useState } from "react";
import { useStatus } from "@/lib/queries";
import { lastMessageAt, onConnectionChange } from "@/lib/ws";
import { useServerTimeZone } from "@/lib/serverTime";

const clock = (ms: number) => new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });

/** `online`, and the time of the last message received while offline (null while online). */
export function useConnection() {
  const [online, setOnline] = useState(true);
  useEffect(() => onConnectionChange(setOnline), []);
  return { online, since: online ? null : lastMessageAt() || null };
}

/** `● host · live` or `Offline since 14:02`, plus the host time zone when it differs from this device. */
export function HostChip({ compact = false }: { compact?: boolean }) {
  const status = useStatus().data;
  const tz = useServerTimeZone();
  const { online, since } = useConnection();
  const host = status?.host || "daemon";
  const zone = tz?.differs ? (status?.time_zone ? `${clock(Date.now())} ${tz.label}` : tz.label) : null;
  return (
    <span className={`inline-flex min-w-0 items-center gap-1.5 whitespace-nowrap ${online ? "text-ink-2" : "font-semibold text-gh-closed"}`} role="status" title={online ? `Connected to ${host}${zone ? ` · host time ${zone}` : ""}` : `Not connected to ${host}; showing what was last received`}>
      <span className={`size-[7px] flex-none rounded-full ${online ? "bg-gh-open" : "bg-gh-closed"}`} />
      <span className="truncate">{online ? `${host}${compact ? "" : " · live"}` : `Offline${since ? ` since ${clock(since)}` : ""}`}</span>
      {online && zone && !compact && <span className="truncate text-ink-3">· {zone}</span>}
    </span>
  );
}
