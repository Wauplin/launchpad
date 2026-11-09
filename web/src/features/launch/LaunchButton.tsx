import { useEffect, useRef, useState, type MouseEvent, type ReactNode } from "react";
import { Icon } from "@/components/Icon";
import { MenuItem, Popover } from "@/components/Popover";
import { ago } from "@/lib/format";
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
const sinceKey = (workspace: string | null) => `launchpad-draft-since-${workspace ?? ""}`;

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

/** Opens the composer with `ctx` and the field's text added, for a launch that needs more choices. */
export function openOptions(ctx: LaunchContext | undefined, text: string) {
  const c = useComposer.getState();
  c.start(ctx);
  const t = text.trim();
  if (t) c.update((s) => ({ ...s, text: s.text.trim() ? `${s.text.trimEnd()}\n\n${t}` : t }));
  c.setMinimized(false);
}

/**
 * Launch in the top-bar field. The main part launches the field's text at once (`onLaunch`; with an empty field or a
 * kept draft it opens the composer). Its menu opens the full composer with the text, for a launch that needs more
 * choices. With a minimized draft it shows a "draft" badge, flashes when + Add buttons add context, and its menu starts
 * over or discards the draft (prominent after 7 days).
 */
export function LaunchSplitButton({ ctx, text, onLaunch, onUsed, primary = false, title }: { ctx?: LaunchContext; text: string; onLaunch: () => void; onUsed: () => void; primary?: boolean; title: string }) {
  const { open, minimized, discard, spec, autoLaunchUntil } = useComposer();
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
  const options = () => { openOptions(ctx, text); onUsed(); };
  const cls = primary ? "btn-primary" : "";
  const draft = open && minimized && !autoLaunchUntil;
  const stale = draft && since !== null && Date.now() - since > STALE_MS;
  const first = spec.text.trim().split("\n")[0];
  const age = since !== null ? ago(since) : null;
  return (
    <span className={`inline-flex rounded-card transition-shadow ${flash ? "ring-[3px] ring-teal" : ""}`} data-tour="topbar.launch">
      <button className={`btn ${cls} rounded-r-none`} title={draft ? `Open the draft${first ? `: ${first}` : ""}${age ? ` (started ${age})` : ""}${text.trim() ? "; your text is added to it" : ""}` : title} onClick={onLaunch}>
        <Icon name="play" size={12} />
        Launch
        {draft && <span className={`rounded-full px-1.5 text-[11px] leading-4 font-semibold ${stale ? "bg-attn-soft text-attn" : "bg-sunk text-ink-2"}`}>draft{items ? ` · ${items}` : ""}</span>}
      </button>
      <Popover align="right" width={280} trigger={({ open: menu, toggle }) => (
        <button className={`btn ${cls} rounded-l-none ${primary ? "border-l-white/30" : "border-l-0"} px-1.5!`} aria-label="Launch options" title="Launch options" aria-expanded={menu} onClick={toggle}><Icon name="chev" size={12} /></button>
      )}>
        {(close) => (
          <>
            {draft
              ? <MenuItem icon={<Icon name="sliders" size={12} />} label="Open the draft" detail={[first || "No prompt yet", age && `started ${age}`].filter(Boolean).join(" · ")} onClick={() => { close(); options(); }} />
              : <MenuItem icon={<Icon name="sliders" size={12} />} label="Launch with options…" detail="Open the composer with this text, for a launch that needs more choices" onClick={() => { close(); options(); }} />}
            {draft && <MenuItem icon={<Icon name="plus" size={12} />} label="New draft instead" detail="Discards the draft" onClick={() => { close(); discard(); options(); }} />}
            {draft && <MenuItem icon={<Icon name="trash" size={12} />} label={stale ? "Discard this old draft" : "Discard draft"} detail={stale ? `Started ${age}` : undefined} onClick={() => { close(); discard(); }} />}
          </>
        )}
      </Popover>
    </span>
  );
}
