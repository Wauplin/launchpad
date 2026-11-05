// Routines (code: crons): runs that start on their own, from a schedule, GitHub activity in the Inbox or another
// agent's turn ending Done. A bot's routine sends its instructions to the bot's thread; other routines launch a
// new session. A run ends with a report, or with `no_update`, which stays silent.

import type { CronAgent } from "@/gen/CronAgent";
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

export function triggerSummary(r: Pick<CronAgent, "trigger" | "schedule">, folderName?: (id: string) => string): string {
  const t: Trigger = r.trigger;
  if (t.kind === "schedule") return `Schedule · ${r.schedule}`;
  if (t.kind === "github") return `GitHub · ${t.events.map((e) => GITHUB_EVENTS.find((x) => x.id === e)?.label.toLowerCase() ?? e).join(", ")} · ${t.repo ?? "the workspace's repositories"}`;
  return `When an agent is done · ${t.folder ? folderName?.(t.folder) ?? "a folder" : "any folder of the workspace"}`;
}

/** The routine's state in a few words: paused, next run, or waiting for its event. */
export function routineState(r: CronAgent, botName?: string): string {
  if (r.bot_paused) return `Paused with ${botName ?? "its bot"}`;
  if (r.paused) return "Paused";
  if (r.trigger.kind === "schedule") return r.next_run_at ? `Next run ${new Date(r.next_run_at).toLocaleString([], { dateStyle: "medium", timeStyle: "short" })}` : "No run planned";
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
