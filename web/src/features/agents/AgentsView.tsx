// Agents workspace (sketch S3): sessions grouped as Done, Running and Closed, shown in 1, 2 or 4 panes.
// Clicking a session loads it in the focused pane.

import { useNavigate, useSearch } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useEffect, useRef, useState } from "react";
import type { Session } from "@/gen/Session";
import { HarnessMark } from "@/components/HarnessMark";
import { isRunning, isDone } from "@/lib/sessionStatus";
import { Icon } from "@/components/Icon";
import { LaunchButton } from "@/features/launch/LaunchButton";
import { projectName, useProjects, useSessions } from "@/lib/queries";
import { sendPresence } from "@/lib/ws";
import { useLayout, type Layout } from "@/stores/layout";
import { SessionPane } from "./SessionPane";
import { StatusBadge } from "./SessionBits";

const SessionRow = memo(function SessionRow({ s, project, selected, onClick }: { s: Session; project: string; selected: boolean; onClick: () => void }) {
  const dim = s.status === "closed";
  return (
    <button
      onClick={onClick}
      className={`grid h-[68px] w-full grid-cols-[18px_1fr] gap-x-[9px] gap-y-[3px] border-b border-line-2 px-3 py-2 text-left hover:bg-[#f5f8fb] ${selected ? "bg-select!" : ""}`}
    >
      <span className="row-span-2 mt-px">
        <HarnessMark harness={s.harness} size={18} />
      </span>
      <span className={`truncate text-[12.5px] leading-[1.35] ${dim ? "font-medium text-ink-2" : "font-semibold"}`} title={s.title}>
        {s.title}
      </span>
      <span className="flex items-center gap-x-2 overflow-hidden whitespace-nowrap text-[11.5px] text-ink-3">
        <span className="max-w-[110px] truncate">{project}</span>
        <StatusBadge s={s} />
        {s.usage.cost_usd != null && <span>${s.usage.cost_usd.toFixed(2)}</span>}
      </span>
    </button>
  );
});

function Group({ title, count, highlight, children }: { title: string; count: number; highlight?: boolean; children?: React.ReactNode }) {
  return (
    <>
      <div className="flex items-center gap-2 px-3 pt-3 pb-1.5 seclabel">
        {title} <b className={highlight && count ? "text-attn" : "font-normal text-ink-3"}>{count}</b>
      </div>
      {children}
    </>
  );
}

export function LayoutSwitch() {
  const { layout, setLayout } = useLayout();
  return (
    <span className="inline-flex gap-0.5 rounded-lg bg-sunk p-0.5">
      {([1, 2, 4] as Layout[]).map((l) => (
        <button
          key={l}
          title={`${l} pane${l > 1 ? "s" : ""}`}
          onClick={() => setLayout(l)}
          className={`grid h-6 w-7 place-items-center rounded-md ${layout === l ? "bg-paper text-ink shadow-[0_1px_2px_rgba(23,32,43,.12),inset_0_0_0_1px_var(--color-line)]" : "text-ink-3"}`}
        >
          <Icon name={l === 1 ? "pane1" : l === 2 ? "pane2" : "pane4"} size={14} />
        </button>
      ))}
    </span>
  );
}

export function AgentsView() {
  const sessions = useSessions().data ?? [];
  const projects = useProjects().data;
  const { layout, panes, focused, focus, show, clearSession } = useLayout();
  const search = useSearch({ strict: false }) as { session?: string };
  const navigate = useNavigate();
  const [closedLimit, setClosedLimit] = useState(30);
  const listRef = useRef<HTMLDivElement>(null);

  // From a desktop notification: /agents?session=<id>.
  useEffect(() => {
    if (search.session) {
      show(search.session);
      void navigate({ to: "/agents", search: {}, replace: true });
    }
  }, [search.session, show, navigate]);

  // Tell the daemon which sessions are on screen, so it does not notify about them.
  const visible = panes.slice(0, layout).filter(Boolean) as string[];
  useEffect(() => {
    const report = () => sendPresence(document.hasFocus() && document.visibilityState === "visible", visible);
    report();
    window.addEventListener("focus", report);
    window.addEventListener("blur", report);
    document.addEventListener("visibilitychange", report);
    return () => {
      window.removeEventListener("focus", report);
      window.removeEventListener("blur", report);
      document.removeEventListener("visibilitychange", report);
      sendPresence(false, []);
    };
  }, [visible.join(",")]);

  const done = sessions.filter((s) => isDone(s)).sort((a, b) => a.status_since - b.status_since);
  const running = sessions.filter((s) => isRunning(s));
  const closed = sessions.filter((s) => !isDone(s) && !isRunning(s));
  const items: ({ kind: "group"; title: string; count: number; highlight?: boolean } | { kind: "session"; session: Session } | { kind: "more" })[] = [
    { kind: "group", title: "Done", count: done.length, highlight: true },
    ...done.map((session) => ({ kind: "session" as const, session })),
    { kind: "group", title: "Running", count: running.length },
    ...running.map((session) => ({ kind: "session" as const, session })),
    { kind: "group", title: "Closed", count: closed.length },
    ...closed.slice(0, closedLimit).map((session) => ({ kind: "session" as const, session })),
    ...(closed.length > closedLimit ? [{ kind: "more" as const }] : []),
  ];
  const virtual = useVirtualizer({ count: items.length, getScrollElement: () => listRef.current, estimateSize: (i) => items[i].kind === "session" ? 68 : 34, getItemKey: (i) => items[i].kind === "session" ? items[i].session.id : `heading-${i}`, overscan: 6, useFlushSync: false });
  const name = (s: Session) => projectName(projects, s.project_id, s.cwd);
  const selected = panes[focused];
  const row = (s: Session) => <SessionRow key={s.id} s={s} project={name(s)} selected={s.id === selected} onClick={() => show(s.id)} />;

  return (
    <div className="flex h-full">
      <div ref={listRef} className="flex w-[280px] flex-none flex-col overflow-y-auto border-r border-line bg-paper">
        {sessions.length === 0 && (
          <div className="grid gap-3 p-4 text-[12.5px] text-ink-2">
            No session yet.
            <span>
              <LaunchButton small />
            </span>
          </div>
        )}
        <div style={{ height: virtual.getTotalSize(), position: "relative", flexShrink: 0 }}>
          {virtual.getVirtualItems().map((v) => {
            const item = items[v.index];
            return <div key={v.key} style={{ position: "absolute", top: 0, left: 0, width: "100%", height: v.size, transform: `translateY(${v.start}px)` }}>
              {item.kind === "session" ? row(item.session) : item.kind === "group" ? <Group title={item.title} count={item.count} highlight={item.highlight} /> :
                <button className="px-3 py-2 text-left text-xs text-teal-ink hover:underline" onClick={() => setClosedLimit((n) => n + 50)}>Show more</button>}
            </div>;
          })}
        </div>
      </div>
      <div
        className={`grid min-w-0 flex-1 gap-px bg-line ${layout === 1 ? "grid-cols-1" : layout === 2 ? "grid-cols-2" : "grid-cols-2 grid-rows-2"}`}
        style={layout === 4 ? { gridTemplateRows: "minmax(0,1fr) minmax(0,1fr)" } : { gridTemplateRows: "minmax(0,1fr)" }}
      >
        {Array.from({ length: layout }, (_, i) => {
          const id = panes[i];
          return (
            <div key={i} className="min-h-0 min-w-0 bg-paper">
              {id ? (
                <SessionPane sessionId={id} focused={layout > 1 && i === focused} onFocus={() => focus(i)} onClear={() => clearSession(id)} />
              ) : (
                <div
                  className={`grid h-full place-items-center text-[12.5px] text-ink-3 ${layout > 1 && i === focused ? "shadow-[inset_0_0_0_2px_var(--color-teal)]" : ""}`}
                  onMouseDown={() => focus(i)}
                >
                  {layout > 1 ? "Click a session on the left to show it here." : "Pick a session on the left, or launch a new one."}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
