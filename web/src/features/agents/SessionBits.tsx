import type { Session } from "@/gen/Session";
import { age, ago } from "@/lib/format";
import type { IconName } from "@/components/Icon";
import { useRunnerTag } from "@/lib/burst";
import { StatusBadge, type StatusTone } from "@/components/ui";
import { isDone, isRunning, isUnseen } from "@/lib/sessionStatus";

/** "12:04" since `ms`. */
export function elapsed(ms: number): string {
  const s = Math.max(0, Math.floor((Date.now() - ms) / 1000));
  const m = Math.floor(s / 60);
  if (m >= 60) return `${Math.floor(m / 60)} h ${m % 60} min`;
  return `${m}:${String(s % 60).padStart(2, "0")}`;
}

export function compact(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(n >= 10_000_000 ? 0 : 1)}M`;
  if (n >= 1000) return `${(n / 1000).toFixed(n >= 100_000 ? 0 : 1)}k`;
  return String(n);
}

/** A session's status in words, with its StatusBadge color (blue running, amber needs you, green Done, red failed). */
export function statusInfo(s: Session): { label: string; tone: StatusTone; icon: IconName | null } {
  switch (s.status) {
    case "starting":
      return { label: "Starting", tone: "running", icon: null };
    case "working":
      return { label: `Working ${elapsed(s.status_since)}`, tone: "running", icon: null };
    case "waiting":
      return s.open_question ? { label: "Question", tone: "needs", icon: "alert" } : { label: `Done ${age(s.status_since)}`, tone: "done", icon: "okcircle" };
    case "background":
      return { label: `Background ${age(s.status_since)}`, tone: "running", icon: null };
    case "closed":
      return { label: `Archived ${ago(s.ended_at ?? s.status_since)}`, tone: "idle", icon: "archive" };
    case "in_terminal":
      return { label: "In terminal", tone: "idle", icon: "term" };
    case "quota_paused":
      return { label: s.quota_pause?.error ? "Quota paused · needs attention" : "Quota paused", tone: "needs", icon: "clock" };
    case "stopped":
      return { label: "Stopped", tone: "idle", icon: "stop" };
    case "interrupted":
      return { label: "Interrupted", tone: "failed", icon: "alert" };
    case "error":
      return s.capacity_retry_at != null ? { label: "Automatic retry pending", tone: "idle", icon: "clock" } : { label: "Failed", tone: "failed", icon: "xcircle" };
  }
}

/** The session's StatusBadge (an amber dot while a finished turn is unseen), plus where it runs when that is not here. */
export function SessionStatus({ s }: { s: Session }) {
  const { label, tone, icon } = statusInfo(s);
  const where = useRunnerTag(s.runner_id);
  const unseen = isUnseen(s) && ((isDone(s) && !s.open_question) || s.status === "error" || s.status === "interrupted");
  // The last turn's branches, files or conversation are not all on Home yet: its runner saves them, and retries.
  const unsaved = s.unsaved && !isRunning(s) && s.status !== "closed" ? s.unsaved : null;
  return (
    <span className="inline-flex items-center gap-1">
      <StatusBadge tone={tone} label={label} icon={icon} unseen={unseen} />
      {unsaved && <span className={`inline-flex h-5 items-center rounded-full px-2 text-[11px] whitespace-nowrap ${unsaved.attempts ? "bg-attn-soft text-attn" : "bg-sunk text-ink-3"}`} title={unsaved.note ? `Not saved on Home: ${unsaved.note}. Its runner retries every few minutes and when it reconnects.` : "Its runner is saving this turn's work on Home."}>{unsaved.attempts ? "Not on Home" : "Saving"}</span>}
      {where && <span className={`inline-flex h-5 items-center rounded-full px-2 text-[11px] whitespace-nowrap ${where.tone === "work" ? "bg-work-soft text-work" : where.tone === "err" ? "bg-diff-del text-gh-closed" : "bg-sunk text-ink-3"}`} title={where.title}>{where.label}</span>}
    </span>
  );
}

/** "182k tokens · 18% context · $1.84" */
export function usageLine(s: Session): string {
  const u = s.usage;
  const parts: string[] = [];
  if (u.context_tokens) {
    parts.push(`${compact(u.context_tokens)} tokens`);
    if (u.context_window) parts.push(`${Math.round((100 * u.context_tokens) / u.context_window)}% context`);
  }
  if (u.cost_usd != null) parts.push(`$${u.cost_usd.toFixed(2)}`);
  return parts.join(" · ");
}
