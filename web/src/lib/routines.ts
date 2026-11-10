// Routines (code: crons): runs that start on their own, from a schedule, GitHub activity in the Inbox or another
// agent's turn ending Done, or Launchpad's own failures. A bot's routine sends its instructions to the bot's thread; other routines launch a
// new session. A run ends with a report, or with `no_update`, which stays silent.

import { when } from "./format.ts";
import type { CronAgent } from "@/gen/CronAgent";
import type { FailureKind } from "@/gen/FailureKind";
import type { GithubEvent } from "@/gen/GithubEvent";
import type { RoutineRun } from "@/gen/RoutineRun";
import type { RunOutcome } from "@/gen/RunOutcome";
import type { Trigger } from "@/gen/Trigger";

export const GITHUB_EVENTS: { id: GithubEvent; label: string }[] = [
  { id: "review_requested", label: "Review requested" },
  { id: "mentioned", label: "Mentioned" },
  { id: "assigned", label: "Assigned" },
  { id: "ci_activity", label: "CI activity" },
  { id: "release", label: "New release" },
  { id: "any", label: "Any activity" },
];

export const FAILURE_EVENTS: { id: FailureKind; label: string }[] = [
  { id: "any", label: "Any failure" },
  { id: "session", label: "Agent failed" },
  { id: "burst", label: "Burst job" },
  { id: "routine", label: "Routine run" },
  { id: "schedule", label: "Scheduled launch" },
  { id: "runner", label: "Runner offline" },
  { id: "home_restart", label: "Home restarted" },
  { id: "quota", label: "Quota reading missing" },
];

export const OUTCOME: Record<RunOutcome, { label: string; tone: "ok" | "quiet" | "bad" | "work" }> = {
  running: { label: "Running", tone: "work" },
  report: { label: "Report", tone: "ok" },
  no_update: { label: "No update", tone: "quiet" },
  skipped: { label: "Skipped", tone: "quiet" },
  failed: { label: "Failed", tone: "bad" },
  interrupted: { label: "Interrupted", tone: "bad" },
  stopped: { label: "Stopped", tone: "quiet" },
};

/** Color of a run in the strip of recent runs. */
export const OUTCOME_CLASS: Record<RunOutcome, string> = {
  running: "bg-teal",
  report: "bg-gh-open",
  no_update: "bg-ink-3/40",
  skipped: "border border-line bg-paper",
  failed: "bg-gh-closed",
  interrupted: "bg-gh-closed",
  stopped: "bg-ink-3/40",
};

/** This browser's IANA time zone: schedules saved here run in it. */
export const browserZone = () => Intl.DateTimeFormat().resolvedOptions().timeZone;

const WEEKDAYS = ["Sundays", "Mondays", "Tuesdays", "Wednesdays", "Thursdays", "Fridays", "Saturdays"];
/** A plain number within [lo, hi], else NaN. */
const num = (v: string, lo: number, hi: number) => {
  const n = /^\d+$/.test(v) ? Number(v) : NaN;
  return n >= lo && n <= hi ? n : NaN;
};
/** The N of `*\/N` when N divides `whole` evenly (so the runs are really N apart), else null. */
const step = (v: string, whole: number) => {
  const n = Number(v.match(/^\*\/(\d+)$/)?.[1]);
  return n > 0 && n < whole && whole % n === 0 ? n : null;
};
const list = (items: string[]) => (items.length < 2 ? items.join("") : `${items.slice(0, -1).join(", ")} and ${items[items.length - 1]}`);
const two = (n: number) => String(n).padStart(2, "0");

/**
 * A five-field cron expression in words: "Every day at 03:00", "Weekdays at 09:00", "Mondays at 09:00",
 * "Every hour", "Every 15 minutes". Expressions it cannot say exactly (ranges, names, steps that do not divide the
 * hour or the day, months) come back unchanged: never a wrong sentence.
 */
export function describeCron(expr: string): string {
  const raw = expr.trim();
  const f = raw.split(/\s+/);
  if (f.length !== 5) return raw;
  const [min, hour, dom, mon, dow] = f;
  if (mon !== "*") return raw;
  const m = num(min, 0, 59);
  if (hour === "*" && dom === "*" && dow === "*") {
    if (min === "*") return "Every minute";
    const every = step(min, 60);
    if (every) return every === 1 ? "Every minute" : `Every ${every} minutes`;
    if (m === 0) return "Every hour";
    return isNaN(m) ? raw : `Every hour at :${two(m)}`;
  }
  if (isNaN(m)) return raw;
  const everyH = step(hour, 24);
  if (everyH && dom === "*" && dow === "*") return `Every ${everyH} hours${m ? ` at :${two(m)}` : ""}`;
  const hours = hour.split(",").map((h) => num(h, 0, 23));
  if (hours.some(isNaN)) return raw;
  const at = `at ${list([...new Set(hours)].sort((a, b) => a - b).map((h) => `${two(h)}:${two(m)}`))}`;
  if (dom === "*" && dow === "*") return `Every day ${at}`;
  if (dom === "*" && dow === "1-5") return `Weekdays ${at}`;
  if (dom === "*") {
    const days = [...new Set(dow.split(",").map((d) => num(d, 0, 7) % 7))].sort((a, b) => a - b);
    if (days.some(isNaN)) return raw;
    if (days.join() === "0,6") return `Weekends ${at}`;
    return `${list(days.map((d) => WEEKDAYS[d]))} ${at}`;
  }
  const day = num(dom, 1, 31);
  if (dow === "*" && !isNaN(day)) return `Every month on day ${day} ${at}`;
  return raw;
}

/** Runs a routine started by events may start in an hour (`crons::EVENT_RUNS_PER_HOUR`). */
export const EVENT_RUNS_PER_HOUR = 6;

/** `time_zone` absent: a record saved before schedules kept a zone runs in the server's time. */
export function triggerSummary(r: Pick<CronAgent, "trigger" | "schedule" | "time_zone">, folderName?: (id: string) => string): string {
  const t: Trigger = r.trigger;
  if (t.kind === "schedule") return `${describeCron(r.schedule)} (${r.time_zone ?? "server time"})`;
  if (t.kind === "github") return `GitHub · ${t.events.map((e) => GITHUB_EVENTS.find((x) => x.id === e)?.label.toLowerCase() ?? e).join(", ")} · ${t.repo ?? "the workspace's repositories"}`;
  if (t.kind === "failure") return `When Launchpad fails · ${t.events.map((e) => FAILURE_EVENTS.find((x) => x.id === e)?.label.toLowerCase() ?? e).join(", ")}`;
  return `When an agent is done · ${t.folder ? folderName?.(t.folder) ?? "a folder" : "any folder of the workspace"}${t.include_bots ? " · bots too" : ""}`;
}

/** Tooltip with the raw cron expression of a schedule trigger. */
export const cronTitle = (r: Pick<CronAgent, "trigger" | "schedule">) => (r.trigger.kind === "schedule" ? `Cron: ${r.schedule}` : undefined);

/** The routine's state in a few words: paused, next run, or waiting for its event. */
export function routineState(r: CronAgent, botName?: string): string {
  if (r.bot_paused) return `Paused with ${botName ?? "its bot"}`;
  if (r.paused) return "Paused";
  if (r.trigger.kind === "schedule") return r.next_run_at ? `Next run ${when(r.next_run_at)}` : "No run planned";
  if (r.trigger.kind === "failure") return "Waits for a failure";
  return r.trigger.kind === "github" ? "Waits for GitHub activity" : "Waits for an agent to finish";
}

/** The last run that did its job (a report or nothing to report). */
export const lastSuccess = (r: CronAgent): RoutineRun | undefined => [...r.history].reverse().find((x) => x.outcome === "report" || x.outcome === "no_update");

/** The first line of a bot routine's message (`crons::bot_message`): `Routine “title” · cause`. */
const HEADER = /^Routine “([^”]+)” · ([^\n]*)\n/;

/** A routine's message in a bot thread: its title, cause and the instructions (without the run rules). */
export function routineMessage(text: string): { title: string; cause: string; body: string } | null {
  const m = text.match(HEADER);
  if (!m) return null;
  const rest = text.slice(m[0].length).trim();
  const cut = rest.indexOf("This is a background run of the routine");
  return { title: m[1], cause: m[2], body: (cut >= 0 ? rest.slice(0, cut) : rest).trim() };
}

/** Whether a final message means "nothing to report" (the same rule as Home's `crons::is_no_update`). */
export const isNoUpdate = (text: string | null | undefined) => !!text && text.trim().replace(/^[`*".\s]+|[`*".\s]+$/g, "").toLowerCase() === "no_update";
