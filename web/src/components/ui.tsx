// Shared UI primitives. Features use these instead of hand-building headers, tabs, filters,
// confirmations, loading/empty/error states, status pills, option tokens or settings rows.

import { useEffect, useRef, useState, type ReactNode } from "react";
import { Icon, type IconName } from "./Icon";

/** Page title (18/600), one meta line, right-aligned actions (one primary, up to two secondary, then ⋯) and optional tabs. */
export function PageHeader({ title, meta, actions, tabs, back, icon, className = "" }: { title: ReactNode; meta?: ReactNode; actions?: ReactNode; tabs?: ReactNode; back?: ReactNode; icon?: ReactNode; className?: string }) {
  return (
    <header className={`page-header border-b border-line-2 bg-paper px-5 pt-3 ${tabs ? "" : "pb-3"} ${className}`}>
      {back && <div className="mb-1 text-[12px]">{back}</div>}
      <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-2">
        <div className="flex min-w-0 flex-1 basis-[240px] items-center gap-2">
          {icon}
          <div className="min-w-0">
            <h1 className="m-0 truncate text-[18px] leading-tight font-semibold">{title}</h1>
            {meta && <div className="mt-0.5 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-[12px] text-ink-3">{meta}</div>}
          </div>
        </div>
        {actions && <div className="flex flex-wrap items-center gap-2">{actions}</div>}
      </div>
      {tabs && <div className="mt-2">{tabs}</div>}
    </header>
  );
}

export type TabItem<T extends string> = { id: T; label: ReactNode; count?: number; disabled?: boolean; title?: string; icon?: IconName };

/** Underline tabs: switch between views of one object (Summary/Trace/Changes, Conversation/Files). */
export function Tabs<T extends string>({ items, value, onChange, label, className = "" }: { items: TabItem<T>[]; value: T; onChange: (id: T) => void; label: string; className?: string }) {
  return (
    <div role="tablist" aria-label={label} className={`lp-tabs phone-tabs ${className}`}>
      {items.map((t) => (
        <button key={t.id} role="tab" type="button" aria-selected={t.id === value} aria-pressed={t.id === value} disabled={t.disabled} title={t.title} className="lp-tab" onClick={() => onChange(t.id)}>
          {t.icon && <Icon name={t.icon} size={12} />}
          {t.label}
          {t.count != null && <span className="lp-count">{t.count}</span>}
        </button>
      ))}
    </div>
  );
}

/** Pill with an optional count: narrows a list (Inbox reasons, PRs/Issues, Review groups). */
export function FilterChip({ active, onClick, children, count, title, disabled }: { active: boolean; onClick: () => void; children: ReactNode; count?: number; title?: string; disabled?: boolean }) {
  return (
    <button type="button" className="lp-chip" aria-pressed={active} title={title} disabled={disabled} onClick={onClick}>
      {children}
      {count != null && <span className="lp-count">{count}</span>}
    </button>
  );
}

export function FilterChips({ label, children, className = "" }: { label: string; children: ReactNode; className?: string }) {
  return <div role="group" aria-label={label} className={`phone-tabs flex flex-wrap items-center gap-1.5 ${className}`}>{children}</div>;
}

/**
 * Destructive or publishing action. The first click arms it and shows the consequence on one line with Cancel;
 * the second click runs it. Never use window.confirm.
 */
export function ConfirmButton({ children, confirmLabel, consequence, onConfirm, tone = "danger", className = "", small = true, disabled, title, icon }: {
  children: ReactNode;
  confirmLabel?: ReactNode;
  consequence: ReactNode;
  onConfirm: () => unknown;
  tone?: "danger" | "publish";
  className?: string;
  small?: boolean;
  disabled?: boolean;
  title?: string;
  icon?: IconName;
}) {
  const [armed, setArmed] = useState(false);
  const [busy, setBusy] = useState(false);
  const ref = useRef<HTMLSpanElement>(null);
  useEffect(() => {
    if (!armed) return;
    const away = (e: PointerEvent) => { if (!ref.current?.contains(e.target as Node)) setArmed(false); };
    const t = setTimeout(() => setArmed(false), 15000);
    document.addEventListener("pointerdown", away);
    return () => { clearTimeout(t); document.removeEventListener("pointerdown", away); };
  }, [armed]);
  const size = small ? "btn-sm" : "";
  if (!armed)
    return <button type="button" className={`btn ${size} ${className}`} disabled={disabled || busy} title={title} onClick={() => setArmed(true)}>{icon && <Icon name={icon} size={12} />}{children}</button>;
  return (
    <span ref={ref} role="group" className="inline-flex max-w-full flex-wrap items-center gap-1.5" aria-live="polite">
      <span className="min-w-0 text-[12px] text-ink-2">{consequence}</span>
      <button
        type="button"
        className={`btn ${size} ${tone === "danger" ? "btn-danger" : "btn-primary"}`}
        disabled={busy}
        autoFocus
        onClick={async () => {
          setBusy(true);
          try { await onConfirm(); } finally { setBusy(false); setArmed(false); }
        }}
      >
        {icon && <Icon name={icon} size={12} />}
        {confirmLabel ?? children}
      </button>
      <button type="button" className={`btn btn-ghost ${size}`} onClick={() => setArmed(false)}>Cancel</button>
    </span>
  );
}

/** Spinner plus the noun being loaded: <Loading what="pull requests" />. */
export function Loading({ what, className = "" }: { what?: string; className?: string }) {
  return (
    <div role="status" className={`flex items-center gap-2 p-4 text-[12px] text-ink-3 ${className}`}>
      <span className="lp-spin" aria-hidden="true" />
      {what ? `Loading ${what}…` : "Loading…"}
    </div>
  );
}

/** Icon, one line and at most one action. `fill` centers it in the available area. */
export function Empty({ icon, children, action, fill, className = "" }: { icon?: IconName; children: ReactNode; action?: ReactNode; fill?: boolean; className?: string }) {
  return (
    <div className={`${fill ? "grid h-full place-items-center" : ""} p-4 ${className}`}>
      <div className={`flex flex-col gap-2 text-[12px] text-ink-3 ${fill ? "items-center text-center" : "items-start"}`}>
        <div className="flex items-center gap-2">
          {icon && <Icon name={icon} size={fill ? 18 : 13} />}
          <span>{children}</span>
        </div>
        {action}
      </div>
    </div>
  );
}

/** Danger-soft banner with human text and an optional Retry. */
export function ErrorState({ error, onRetry, action, className = "" }: { error: unknown; onRetry?: () => void; action?: ReactNode; className?: string }) {
  return (
    <div role="alert" className={`flex flex-wrap items-center gap-2 rounded-lg border border-danger/40 bg-diff-del px-3 py-2 text-[12px] text-danger-ink ${className}`}>
      <Icon name="alert" size={13} />
      <span className="min-w-0 flex-1 break-words">{humanError(error)}</span>
      {action}
      {onRetry && <button type="button" className="btn btn-sm" onClick={onRetry}><Icon name="refresh" size={12} />Retry</button>}
    </div>
  );
}

/** Daemon and OS error text, rewritten for people. Unknown messages pass through unchanged. */
export function humanError(error: unknown): string {
  const text = error instanceof Error ? error.message : typeof error === "string" ? error : String(error);
  if (/No such file or directory|os error 2\b|does not exist/i.test(text)) {
    const path = text.match(/(?:^|\s)((?:[\w.-]+:)?\/[^\s:]+)/)?.[1];
    return path ? `The folder ${path} no longer exists. It may have been removed after a merge.` : "This folder or file no longer exists. It may have been removed after a merge.";
  }
  if (/Permission denied|os error 13\b/i.test(text)) return "Launchpad does not have permission to open this folder or file.";
  if (/Failed to fetch|NetworkError|Load failed/i.test(text)) return "Launchpad cannot reach the daemon. Check the connection and try again.";
  return text;
}

export type BadgeTone = "work" | "attn" | "done" | "neutral" | "danger" | "ok" | "teal";

const BADGE: Record<BadgeTone, string> = {
  work: "bg-work-soft text-work",
  attn: "bg-attn-soft text-attn",
  done: "bg-sunk text-ink-3",
  neutral: "bg-sunk text-ink-2",
  danger: "bg-diff-del text-gh-closed",
  ok: "bg-diff-add text-gh-open",
  teal: "bg-teal-soft text-teal-ink",
};

/** The one pill shape for states (Running, Done, Question, Failed, Quota paused, counts). */
export function Badge({ tone = "neutral", icon, dot, children, title, className = "" }: { tone?: BadgeTone; icon?: IconName; dot?: boolean; children: ReactNode; title?: string; className?: string }) {
  return (
    <span title={title} className={`inline-flex h-5 items-center gap-[5px] rounded-full px-2 text-[11px] font-semibold whitespace-nowrap ${BADGE[tone]} ${className}`}>
      {icon ? <Icon name={icon} size={11} /> : dot ? <span className="size-1.5 rounded-full bg-current" /> : null}
      {children}
    </span>
  );
}

/** The one token shape for options (profile, worktree, warnings) and folder chips. */
export function Token({ tone = "neutral", icon, children, title, className = "" }: { tone?: "neutral" | "warn" | "teal"; icon?: IconName; children: ReactNode; title?: string; className?: string }) {
  const color = tone === "warn" ? "border-attn-line bg-attn-soft text-attn" : tone === "teal" ? "border-teal/40 bg-teal-soft text-teal-ink" : "border-line-2 bg-chrome text-ink-2";
  return (
    <span title={title} className={`inline-flex h-5 max-w-full items-center gap-1 rounded-[5px] border px-1.5 text-[11px] whitespace-nowrap ${color} ${className}`}>
      {icon && <Icon name={icon} size={11} />}
      <span className="truncate">{children}</span>
    </span>
  );
}

export type SettingScope = "device" | "all";

/** "This device" (browser storage) or "All devices" (daemon config). */
export function ScopeBadge({ scope }: { scope: SettingScope }) {
  return scope === "device"
    ? <Badge tone="neutral" title="Stored in this browser only">This device</Badge>
    : <Badge tone="teal" title="Stored by the daemon; applies to every paired browser">All devices</Badge>;
}

/** Inline save feedback for save-on-change fields. */
export type SaveState = "idle" | "saving" | "saved" | "error";

export function SavedMark({ state, error }: { state: SaveState; error?: string }) {
  if (state === "idle") return null;
  if (state === "saving") return <span className="text-[11px] text-ink-3" aria-live="polite">Saving…</span>;
  if (state === "error") return <span className="text-[11px] text-danger-ink" role="alert">{error ?? "Not saved"}</span>;
  return <span className="inline-flex items-center gap-1 text-[11px] text-gh-open" aria-live="polite"><Icon name="check" size={11} />Saved</span>;
}

/** Runs `save` and exposes the inline state; "Saved ✓" fades after two seconds. */
export function useSaveState() {
  const [state, setState] = useState<SaveState>("idle");
  const [error, setError] = useState<string>();
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);
  const run = async (save: () => Promise<unknown>) => {
    clearTimeout(timer.current);
    setState("saving");
    try {
      await save();
      setState("saved");
      timer.current = setTimeout(() => setState("idle"), 2000);
    } catch (e) {
      setError(humanError(e));
      setState("error");
    }
  };
  return { state, error, run };
}

/** Label, one-line hint, control, scope badge and inline save state. */
export function SettingRow({ label, hint, info, scope, state, error, children, htmlFor }: { label: ReactNode; hint?: ReactNode; info?: string; scope?: SettingScope; state?: SaveState; error?: string; children: ReactNode; htmlFor?: string }) {
  return (
    <div className="phone-setting-row grid grid-cols-[minmax(0,240px)_minmax(0,1fr)] items-start gap-x-6 gap-y-1 border-b border-line-2 py-3 last:border-b-0">
      <div className="min-w-0">
        <div className="flex flex-wrap items-center gap-2">
          <label htmlFor={htmlFor} className="text-[13px] font-semibold">{label}</label>
          {info && <span className="cursor-help text-ink-3" title={info} aria-label={info}>ⓘ</span>}
          {scope && <ScopeBadge scope={scope} />}
        </div>
        {hint && <div className="mt-0.5 text-[12px] text-ink-3">{hint}</div>}
      </div>
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        {children}
        {state && <SavedMark state={state} error={error} />}
      </div>
    </div>
  );
}
