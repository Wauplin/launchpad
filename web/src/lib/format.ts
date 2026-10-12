const DAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

function hm(d: Date) {
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

/** Reset time: "16:40" today, "Mon 09:00" within a week, "12 Oct" later. */
export function formatReset(unixSeconds: number): string {
  // The usage endpoint returns times like 13:29:59.7: round to the minute.
  const d = new Date(Math.round(unixSeconds / 60) * 60_000);
  const now = new Date();
  if (d.toDateString() === now.toDateString()) return hm(d);
  if (d.getTime() - now.getTime() < 6 * 86400_000) return `${DAYS[d.getDay()]} ${hm(d)}`;
  return d.toLocaleDateString("en-GB", { day: "numeric", month: "short" });
}

/** "resets Mon 09:00", or "not started" for a quota window without usage since its last reset. */
export function resetLabel(resetsAt: number | null): string {
  return resetsAt === null ? "not started" : `resets ${formatReset(resetsAt)}`;
}

/** "12 s", "4 min", "3 h", "yesterday", "5 d". */
export function formatAge(ms: number): string {
  const s = Math.max(0, Math.round((Date.now() - ms) / 1000));
  if (s < 60) return `${s} s`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m} min`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h} h`;
  const d = Math.round(h / 24);
  return d === 1 ? "yesterday" : `${d} d`;
}
