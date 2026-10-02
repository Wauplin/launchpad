import type { MouseEvent, ReactNode } from "react";
import { Icon } from "@/components/Icon";
import { useComposer, type LaunchContext } from "@/stores/composer";

/** A Launch button: opens the composer with `ctx` added. */
export function LaunchButton({
  ctx,
  label = "Launch",
  small,
  primary = true,
  title = "Launch an agent",
  children,
}: {
  ctx?: LaunchContext;
  label?: string | null;
  small?: boolean;
  primary?: boolean;
  title?: string;
  children?: ReactNode;
}) {
  const start = useComposer((s) => s.start);
  const onClick = (e: MouseEvent) => {
    e.stopPropagation();
    start(ctx);
  };
  return (
    <button className={`btn ${small ? "btn-sm" : ""} ${primary ? "btn-primary" : ""}`} title={title} onClick={onClick}>
      <Icon name="play" size={small ? 12 : 14} />
      {label}
      {children}
    </button>
  );
}
