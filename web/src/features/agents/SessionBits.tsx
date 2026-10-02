import type { Session } from "@/gen/Session";
import { formatAge } from "@/lib/format";

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

const STYLES = {
  work: "bg-work-soft text-work",
  needs: "bg-attn-soft text-attn",
  bg: "bg-paper text-attn shadow-[inset_0_0_0_1px_var(--color-attn-line)]",
  done: "bg-sunk text-ink-2",
  int: "bg-paper text-ink-2 shadow-[inset_0_0_0_1px_var(--color-line)]",
  err: "bg-[#ffebe9] text-gh-closed",
};

export function statusInfo(s: Session): { label: string; style: keyof typeof STYLES } {
  switch (s.status) {
    case "starting":
      return { label: "Starting", style: "work" };
    case "working":
      return { label: `Working ${elapsed(s.status_since)}`, style: "work" };
    case "waiting":
      return { label: s.open_question ? "Question" : `Waiting ${formatAge(s.status_since)}`, style: "needs" };
    case "background":
      return { label: `Background ${formatAge(s.status_since)}`, style: "bg" };
    case "closed":
      return { label: `Closed ${formatAge(s.ended_at ?? s.status_since)} ago`, style: "done" };
    case "in_terminal":
      return { label: "In terminal", style: "int" };
    case "interrupted":
      return { label: "Interrupted", style: "int" };
    case "error":
      return { label: "Error", style: "err" };
  }
}

export function StatusBadge({ s }: { s: Session }) {
  const { label, style } = statusInfo(s);
  return (
    <span className={`inline-flex h-5 items-center gap-[5px] rounded-full px-2 text-[11.5px] font-semibold whitespace-nowrap ${STYLES[style]}`}>
      <span className={`size-1.5 rounded-full ${style === "bg" || style === "int" ? "border-[1.5px] border-current" : "bg-current"}`} />
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
  else if (s.model) parts.push([s.model, s.effort].filter(Boolean).join(" "));
  return parts.join(" · ");
}
