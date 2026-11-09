// The only date helpers of the UI: `ago` for past moments, `age` for the compact form of dense lists and `when` for
// exact or future moments; `time`, `day` and `month` for labels that need only one part (a clock, a chart axis).
// Every helper takes Unix milliseconds. Never show seconds or the current year.

const DAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const DAY_MS = 86_400_000;

const midnight = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
/** Calendar days from today to `d`: 0 today, -1 yesterday, 1 tomorrow. */
const dayOffset = (d: Date) => Math.round((midnight(d) - midnight(new Date())) / DAY_MS);
const year = (d: Date) => (d.getFullYear() === new Date().getFullYear() ? "" : `, ${d.getFullYear()}`);
/** Locale time with hour and minute, never seconds: "3:00 AM" (en-US), "03:00" (en-GB). */
const clock = (d: Date) => d.toLocaleTimeString([], { timeStyle: "short" });

/** Compact age for dense lists: "just now", "5 min", "3 h", "yesterday", "Oct 3" ("Oct 3, 2025" in another year). */
export function age(ms: number): string {
  const s = Math.max(0, Math.round((Date.now() - ms) / 1000));
  if (s < 45) return "just now";
  const m = Math.max(1, Math.round(s / 60));
  if (m < 60) return `${m} min`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h} h`;
  const d = new Date(ms);
  if (dayOffset(d) === -1) return "yesterday";
  return `${MONTHS[d.getMonth()]} ${d.getDate()}${year(d)}`;
}

/** A past moment: "just now", "5 min ago", "3 h ago", "yesterday", "Oct 3". Callers never add " ago". */
export function ago(ms: number): string {
  const a = age(ms);
  return a.endsWith(" min") || a.endsWith(" h") ? `${a} ago` : a;
}

/** An exact or future moment: "today 15:00", "tomorrow 03:00", "yesterday 18:20", "Fri 7 Nov, 03:00". */
export function when(ms: number): string {
  const d = new Date(ms);
  const offset = dayOffset(d);
  if (offset === 0) return `today ${clock(d)}`;
  if (offset === 1) return `tomorrow ${clock(d)}`;
  if (offset === -1) return `yesterday ${clock(d)}`;
  return `${DAYS[d.getDay()]} ${d.getDate()} ${MONTHS[d.getMonth()]}${d.getFullYear() === new Date().getFullYear() ? "" : ` ${d.getFullYear()}`}, ${clock(d)}`;
}

/** Hour and minute only, for a moment of today: "14:02" ("Offline since 14:02"). */
export const time = (ms: number): string => clock(new Date(ms));

/** A calendar day without the time: "Oct 3" ("Oct 3, 2025" in another year). */
export function day(ms: number): string {
  const d = new Date(ms);
  return `${MONTHS[d.getMonth()]} ${d.getDate()}${year(d)}`;
}

/** A month in English like the other labels: "November" ("November 2025" in another year). */
export function month(ms: number): string {
  const d = new Date(ms);
  return `${d.toLocaleDateString("en-US", { month: "long" })}${d.getFullYear() === new Date().getFullYear() ? "" : ` ${d.getFullYear()}`}`;
}

/** Quota reset (Unix seconds, rounded to the minute: the usage endpoint returns times like 13:29:59.7). */
export const formatReset = (unixSeconds: number): string => when(Math.round(unixSeconds / 60) * 60_000);

/** "resets today 16:40", or "not started" for a quota window without usage since its last reset. */
export function resetLabel(resetsAt: number | null): string {
  return resetsAt === null ? "not started" : `resets ${formatReset(resetsAt)}`;
}
