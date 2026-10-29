import type { Session } from "@/gen/Session";
import { formatAge } from "@/lib/format";
import { Icon, type IconName } from "@/components/Icon";

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

// Every status badge has the same shape; only the color and the icon change.
export const STYLES = {
  work: "bg-work-soft text-work",
  needs: "bg-attn-soft text-attn",
  bg: "bg-attn-soft text-attn",
  done: "bg-sunk text-ink-3",
  int: "bg-sunk text-ink-2",
  err: "bg-diff-del text-gh-closed",
};

export function statusInfo(s: Session): { label: string; style: keyof typeof STYLES; icon: IconName | null } {
  switch (s.status) {
    case "starting":
      return { label: "Starting", style: "work", icon: null };
    case "working":
      return { label: `Working ${elapsed(s.status_since)}`, style: "work", icon: null };
    case "waiting":
      return s.open_question ? { label: "Question", style: "needs", icon: "alert" } : { label: `Done ${formatAge(s.status_since)}`, style: "needs", icon: "okcircle" };
    case "background":
      return { label: `Background ${formatAge(s.status_since)}`, style: "work", icon: null };
    case "closed":
      return { label: `Archived ${formatAge(s.ended_at ?? s.status_since)} ago`, style: "done", icon: "archive" };
    case "in_terminal":
      return { label: "In terminal", style: "int", icon: "term" };
    case "quota_paused":
      return { label: s.quota_pause?.error ? "Quota paused · needs attention" : "Quota paused", style: "bg", icon: "clock" };
    case "stopped":
      return { label: "Stopped", style: "int", icon: "stop" };
    case "interrupted":
      return { label: "Interrupted", style: "int", icon: "alert" };
    case "error":
      return s.capacity_retry_at != null ? { label: "Automatic retry pending", style: "bg", icon: "clock" } : { label: "Failed", style: "err", icon: "xcircle" };
  }
}

export function StatusBadge({ s }: { s: Session }) {
  const { label, style, icon } = statusInfo(s);
  return (
    <span className={`inline-flex h-5 items-center gap-[5px] rounded-full px-2 text-[12px] font-semibold whitespace-nowrap ${STYLES[style]}`}>
      {icon ? <Icon name={icon} size={11} /> : <span className="size-1.5 rounded-full bg-current" />}
      {label}
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
