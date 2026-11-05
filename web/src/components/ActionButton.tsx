import { useState, type ReactNode } from "react";
import { action, actionConfirm, runAction, type ActionName, type ActionParams } from "@/lib/actions";
import { ConfirmButton } from "./ui";
import { Icon, type IconName } from "./Icon";
import { toastError } from "./Toast";

/**
 * A button that runs one action of the action list (crates/core/src/actions.rs): its label, icon, confirmation
 * and API call come from the list. Actions with a confirmation use ConfirmButton.
 */
export function ActionButton<T = unknown>({ name, params, vars, body, query, before, onDone, onError, children, consequence: override, confirmLabel, primary, small = true, className = "", disabled, title }: {
  name: ActionName;
  /** The route's `{param}` values; they also fill the confirmation. */
  params?: ActionParams;
  /** Other values of the confirmation text, such as `{channel}`. */
  vars?: Record<string, string>;
  body?: Record<string, unknown> | (() => Record<string, unknown>);
  query?: Record<string, string>;
  /** Runs first, for example to save an edit; the action does not run if it throws. */
  before?: () => Promise<unknown>;
  onDone?: (result: T) => unknown;
  /** Defaults to a toast. */
  onError?: (e: unknown) => void;
  /** Replaces the label from the list. */
  children?: ReactNode;
  /** Replaces the list's confirmation text, for a state that needs a stronger warning (an uncertain send). */
  consequence?: string;
  confirmLabel?: ReactNode;
  primary?: boolean;
  small?: boolean;
  className?: string;
  disabled?: boolean;
  title?: string;
}) {
  const a = action(name);
  const [busy, setBusy] = useState(false);
  const run = async () => {
    setBusy(true);
    try {
      if (before) await before();
      const result = await runAction<T>(name, params, typeof body === "function" ? body() : body, query);
      await onDone?.(result);
    } catch (e) { (onError ?? toastError)(e); }
    finally { setBusy(false); }
  };
  const icon = (a.icon ?? undefined) as IconName | undefined;
  const label = children ?? a.label;
  const listed = actionConfirm(name, { ...vars, ...params });
  const consequence = listed && (override ?? listed);
  if (consequence)
    return <ConfirmButton tone={a.effect === "outside" ? "publish" : "danger"} small={small} className={`${primary ? "btn-primary" : ""} ${className}`} disabled={disabled || busy} title={title ?? a.help} icon={icon} consequence={consequence} confirmLabel={confirmLabel} onConfirm={run}>{label}</ConfirmButton>;
  return (
    <button type="button" className={`btn ${small ? "btn-sm" : ""} ${primary ? "btn-primary" : ""} ${className}`} disabled={disabled || busy} title={title ?? a.help} onClick={() => void run()}>
      {icon && <Icon name={icon} size={12} />}{label}
    </button>
  );
}
