// Time zone of the daemon host (cron schedules use it) compared with the time zone of this browser.

import { useStatus } from "@/app/StatusBar";

/** Minutes east of UTC in `timeZone` at `at`, or null when the browser does not know the zone. */
function offsetAt(timeZone: string, at: number): number | null {
  try {
    const parts = new Intl.DateTimeFormat("en-US", { timeZone, hourCycle: "h23", year: "numeric", month: "numeric", day: "numeric", hour: "numeric", minute: "numeric" }).formatToParts(at);
    const get = (type: string) => Number(parts.find((p) => p.type === type)?.value ?? 0);
    const wall = Date.UTC(get("year"), get("month") - 1, get("day"), get("hour"), get("minute"));
    return Math.round((wall - Math.floor(at / 60_000) * 60_000) / 60_000);
  } catch {
    return null;
  }
}

function formatOffset(minutes: number) {
  const sign = minutes < 0 ? "-" : "+";
  const m = Math.abs(minutes);
  return `UTC${sign}${String(Math.floor(m / 60)).padStart(2, "0")}:${String(m % 60).padStart(2, "0")}`;
}

export type ServerTimeZone = {
  /** IANA name, or the current UTC offset when the host exposes no name. */
  label: string;
  /** IANA name usable with Intl, when the browser knows it. */
  zone: string | null;
  /** The host and this browser use different time zones (different offsets now or across the year). */
  differs: boolean;
};

export function useServerTimeZone(): ServerTimeZone | null {
  const status = useStatus().data;
  if (!status) return null;
  const browser = Intl.DateTimeFormat().resolvedOptions().timeZone;
  const name = status.time_zone;
  const now = Date.now();
  if (name && offsetAt(name, now) !== null) {
    if (name === browser) return { label: name, zone: name, differs: false };
    const year = new Date(now).getFullYear();
    const samples = [now, Date.UTC(year, 0, 15), Date.UTC(year, 6, 15)];
    const local = (at: number) => -new Date(at).getTimezoneOffset();
    return { label: name, zone: name, differs: samples.some((at) => offsetAt(name, at) !== local(at)) };
  }
  const label = name ?? formatOffset(status.utc_offset_minutes);
  return { label, zone: null, differs: status.utc_offset_minutes !== -new Date(now).getTimezoneOffset() };
}

/** The wall-clock time of `at` on the daemon host, for hints next to times entered in this browser. */
export function serverTimeOf(tz: ServerTimeZone, at: number): string | null {
  if (!tz.zone) return null;
  return new Date(at).toLocaleString(undefined, { timeZone: tz.zone, weekday: "short", hour: "2-digit", minute: "2-digit" });
}
