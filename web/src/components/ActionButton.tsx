import { useState, type ReactNode } from "react";
import { action, actionConfirm, runAction, type ActionName, type ActionParams } from "@/lib/actions";
import { ConfirmButton, MenuSection } from "./ui";
import { MenuItem } from "./Popover";
import { Icon, type IconName } from "./Icon";
import { toastError } from "./Toast";

/**
 * A button that runs one action of the action list (crates/core/src/actions.rs): its label, icon, confirmation
 * and API call come from the list. Actions with a confirmation use ConfirmButton.
 */
export function ActionButton<T = unknown>({ name, params, vars, body, query, before, onDone, onError, children, consequence: override, confirmLabel, primary, small = true, className = "", disabled, title, icon: iconOverride }: {
  name: ActionName;
  /** The route's `{param}` values; they also fill the confirmation. */
  params?: ActionParams;
  /** Other values of the confirmation text, such as `{connection}`. */
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
  /** Replaces the list's icon; null: none. */
  icon?: IconName | null;
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
  const icon = (iconOverride === null ? undefined : iconOverride ?? a.icon ?? undefined) as IconName | undefined;
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

/**
 * One action of the action list as an item of a ⋯ menu (RowActions). The menu closes first, then the action runs.
 * An action with a confirmation shows its ConfirmButton at the bottom of the menu instead (put those last).
 */
export function ActionMenuItem<T = unknown>({ name, params, vars, body, query, close, onDone, label, detail, icon: iconOverride }: {
  name: ActionName;
  params?: ActionParams;
  vars?: Record<string, string>;
  body?: Record<string, unknown>;
  query?: Record<string, string>;
  close: () => void;
  onDone?: (result: T) => unknown;
  label?: ReactNode;
  detail?: ReactNode;
  icon?: IconName | null;
}) {
  const a = action(name);
  if (actionConfirm(name, { ...vars, ...params }))
    return <MenuSection><ActionButton name={name} params={params} vars={vars} body={body} query={query} onDone={async (r: T) => { close(); await onDone?.(r); }}>{label}</ActionButton></MenuSection>;
  const icon = (iconOverride === null ? undefined : iconOverride ?? a.icon ?? undefined) as IconName | undefined;
  return <MenuItem icon={icon && <Icon name={icon} size={14} />} label={label ?? a.label} detail={detail} onClick={() => {
    close();
    runAction<T>(name, params, body, query).then((r) => onDone?.(r)).catch(toastError);
  }} />;
}
