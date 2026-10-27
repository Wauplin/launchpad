import { useEffect, useRef, useState, type MouseEvent, type ReactNode } from "react";
import { Icon } from "@/components/Icon";
import { MenuItem, Popover } from "@/components/Popover";
import { formatAge } from "@/lib/format";
import { useComposer, type LaunchContext } from "@/stores/composer";
import { useWorkspace } from "@/stores/workspace";

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

const STALE_MS = 7 * 86_400_000;
const sinceKey = (workspace: string | null) => `launchpad-draft-since-${workspace ?? "all"}`;

/** When this browser first saw the current draft (per workspace), for its age. */
function useDraftSince(open: boolean): number | null {
  const workspace = useWorkspace((s) => s.current) ?? null;
  const [since, setSince] = useState<number | null>(null);
  useEffect(() => {
    const key = sinceKey(workspace);
    try {
      if (!open) { localStorage.removeItem(key); setSince(null); return; }
      const saved = Number(localStorage.getItem(key));
      if (saved > 0) setSince(saved);
      else { const now = Date.now(); localStorage.setItem(key, String(now)); setSince(now); }
    } catch { setSince(null); }
  }, [open, workspace]);
  return since;
}

/** Items in the draft: attachments, GitHub items, file lines, context folders and traces. */
export function useDraftItems() {
  return useComposer((s) => (s.spec.attachments ?? []).length + s.spec.github_refs.length + (s.spec.files ?? []).length + s.spec.context_projects.length + (s.spec.agent_traces ?? []).length);
}

/**
 * The one Launch control of the top bar. With a minimized draft it shows a "draft" badge, opens that draft,
 * flashes when + Add buttons add context, and its menu starts over or discards the draft (prominent after 7 days).
 */
export function TopLaunchButton({ ctx }: { ctx?: LaunchContext }) {
  const { open, minimized, setMinimized, start, discard, spec } = useComposer();
  const items = useDraftItems();
  const since = useDraftSince(open);
  const [flash, setFlash] = useState(false);
  const last = useRef(items);
  useEffect(() => {
    const grew = items > last.current;
    last.current = items;
    if (!grew || !minimized) return;
    setFlash(true);
    const t = setTimeout(() => setFlash(false), 900);
    return () => clearTimeout(t);
  }, [items, minimized]);
  if (!open || !minimized) return <LaunchButton ctx={ctx} />;
  const stale = since !== null && Date.now() - since > STALE_MS;
  const first = spec.text.trim().split("\n")[0];
  const age = since !== null ? `${formatAge(since)} ago` : null;
  return (
    <span className={`inline-flex rounded-[8px] transition-shadow ${flash ? "ring-[3px] ring-teal" : ""}`}>
      <button className="btn btn-primary rounded-r-none" title={`Open the draft${first ? `: ${first}` : ""}${age ? ` (started ${age})` : ""}`} onClick={() => setMinimized(false)}>
        <Icon name="play" size={14} />
        Launch
        <span className={`rounded-full px-1.5 text-[11px] leading-4 font-semibold ${stale ? "bg-attn-soft text-attn" : "bg-white/25 text-white"}`}>draft{items ? ` · ${items}` : ""}</span>
      </button>
      <Popover align="right" width={280} trigger={({ open: menu, toggle }) => (
        <button className="btn btn-primary rounded-l-none border-l-white/30 px-1.5!" aria-label="Draft actions" aria-expanded={menu} onClick={toggle}><Icon name="chev" size={12} /></button>
      )}>
        {(close) => (
          <>
            <MenuItem icon={<Icon name="play" size={13} />} label="Open the draft" detail={[first || "No prompt yet", age && `started ${age}`].filter(Boolean).join(" · ")} onClick={() => { close(); setMinimized(false); }} />
            <MenuItem icon={<Icon name="plus" size={13} />} label="New agent instead" detail="Discards the draft" onClick={() => { close(); discard(); start(ctx); }} />
            <MenuItem icon={<Icon name="trash" size={13} />} label={stale ? "Discard this old draft" : "Discard draft"} detail={stale ? `Started ${age}` : undefined} onClick={() => { close(); discard(); }} />
          </>
        )}
      </Popover>
    </span>
  );
}
