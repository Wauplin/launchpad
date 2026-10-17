// Now (phone): what needs you first, with cards that open the agent, then what waits to be published and what
// runs. Uses the same predicates and workspace scope as the desktop Agents list and To publish.

import { useQuery } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import type { TerminalInfo } from "@/gen/TerminalInfo";
import type { Session } from "@/gen/Session";
import type { SessionGitStatus } from "@/gen/SessionGitStatus";
import { HarnessMark } from "@/components/HarnessMark";
import { Icon } from "@/components/Icon";
import { toastError } from "@/components/Toast";
import { AskCard } from "@/features/agents/AskCard";
import { buildRows } from "@/features/agents/rows";
import { StatusBadge } from "@/features/agents/SessionBits";
import { usePublishItems } from "@/features/publish/items";
import { useConnection } from "@/app/Connection";
import { cachedAt } from "@/lib/offlineCache";
import { NotificationsPrompt } from "@/app/NotificationsPrompt";
import { api } from "@/lib/api";
import { formatAge } from "@/lib/format";
import { projectName, useProjects, useSessions } from "@/lib/queries";
import { emptyLog, useSessionLogs } from "@/lib/sessionStore";
import { isDone, isQuotaPaused, isRunning, needsYou } from "@/lib/sessionStatus";
import { useScope } from "@/lib/workspace";
import { terminalStatus, useScopedTerminals } from "@/features/terminals/lib";
import { useLayout } from "@/stores/layout";

const open = (navigate: ReturnType<typeof useNavigate>, id: string) => void navigate({ to: "/agents", search: { session: id } });

function Section({ title, count, children }: { title: string; count: number; children: ReactNode }) {
  return <section className="grid min-w-0 grid-cols-1 gap-2"><h2 className="seclabel px-1">{title} · {count}</h2>{children}</section>;
}

const TerminalRow = memo(function TerminalRow({ terminal, project }: { terminal: TerminalInfo; project: string }) {
  const setDock = useLayout((s) => s.setDock);
  return <Link to="/terminals" onClick={() => setDock({ open: true, active: terminal.id })} className="flex h-16 min-w-0 items-center gap-2.5 px-3 text-ink no-underline active:bg-sunk" title={`${terminal.title} · ${terminal.cwd}`}>
    <Icon name="term" size={18} className="flex-none text-ink-3" />
    <span className="min-w-0 flex-1">
      <span className="block truncate font-medium">{terminal.title}</span>
      <span className="block truncate text-[12.5px] text-ink-3">{project} · {terminal.kind === "tui" ? "TUI" : "Shell"} · {terminalStatus(terminal)}</span>
    </span>
    <Icon name="chev" size={14} className="flex-none -rotate-90 text-ink-3" />
  </Link>;
});

function OpenTerminals() {
  const terminals = useScopedTerminals();
  const projects = useProjects().data;
  const scroll = useRef<HTMLDivElement>(null);
  const list = useVirtualizer({
    count: terminals.length,
    getScrollElement: () => scroll.current,
    estimateSize: () => 64,
    getItemKey: (index) => terminals[index].id,
    overscan: 2,
    useFlushSync: false,
  });
  if (!terminals.length) return null;
  return <Section title="Open terminals" count={terminals.length}>
    <div ref={scroll} className="overflow-y-auto rounded-xl border border-line bg-paper" style={{ height: Math.min(terminals.length * 64, 256) + 2 }}>
      <div className="relative" style={{ height: list.getTotalSize() }}>
        {list.getVirtualItems().map((row) => {
          const terminal = terminals[row.index];
          return <div key={row.key} className="absolute top-0 left-0 w-full" style={{ transform: `translateY(${row.start}px)` }}>
            <TerminalRow terminal={terminal} project={projectName(projects, terminal.project_id, terminal.cwd)} />
          </div>;
        })}
      </div>
    </div>
  </Section>;
}

// Cover the card with a real link; action controls sit above it and leave gaps clickable.
function AgentCard({ s, className, children }: { s: Session; className: string; children: ReactNode }) {
  return <article className={`relative grid min-w-0 grid-cols-1 gap-2.5 rounded-xl border bg-paper p-3 ${className}`}>
    <Link to="/agents" search={{ session: s.id }} aria-label={`Open agent: ${s.title}`} className="absolute inset-0 z-10 rounded-xl focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-teal" />
    {children}
  </article>;
}

function CardHead({ s, project }: { s: Session; project: string }) {
  return <div className="grid min-w-0 grid-cols-[auto_minmax(0,1fr)] items-start gap-x-2.5 gap-y-1.5">
    <span className="mt-0.5"><HarnessMark harness={s.harness} size={18} /></span>
    <div className="min-w-0">
      <div className="line-clamp-2 text-[15px] font-semibold" title={s.title}>{s.title}</div>
      <div className="flex min-w-0 items-center gap-1.5 text-[12.5px] text-ink-3"><span className="truncate">{project}</span><span className="flex-none">· {formatAge(s.status_since)} ago</span></div>
    </div>
    <div className="col-start-2 min-w-0"><StatusBadge s={s} /></div>
  </div>;
}

/** Loads the newest events of a session (the same tail page the session view uses). */
function useTail(id: string) {
  const log = useSessionLogs((st) => st.logs[id]);
  const load = useSessionLogs((st) => st.load);
  useEffect(() => {
    if (log) return;
    useSessionLogs.setState((st) => ({ logs: { ...st.logs, [id]: emptyLog() } }));
    void load(id);
  }, [id, log, load]);
  return log;
}

const QuestionCard = memo(function QuestionCard({ s, project }: { s: Session; project: string }) {
  const navigate = useNavigate();
  const log = useTail(s.id);
  const ask = useMemo(() => {
    const rows = log ? buildRows(log) : [];
    for (let i = rows.length - 1; i >= 0; i--) { const r = rows[i]; if (r.t === "ask") return r.answers ? null : r; }
    return null;
  }, [log]);
  return <AgentCard s={s} className="phone-review-card border-attn-line">
    <CardHead s={s} project={project} />
    {ask && s.live ? <div className="pointer-events-none relative z-20 [&_button]:pointer-events-auto"><AskCard row={ask} sessionId={s.id} live={s.live} hint="Or tap Other answer to write one." /></div> : <p className="text-[13.5px] text-ink-2">{s.last_message ?? "The agent asked a question."}</p>}
    <div className="pointer-events-none relative z-20 flex flex-wrap gap-2 [&_button]:pointer-events-auto"><button className="btn" onClick={() => open(navigate, s.id)}>Other answer…</button></div>
  </AgentCard>;
});

function GitFacts({ id }: { id: string }) {
  const git = useQuery({ queryKey: ["session-git", id], queryFn: () => api.get<SessionGitStatus>(`/api/sessions/${id}/git`), staleTime: 30_000, retry: false });
  const d = git.data;
  if (!d) return null;
  const parts: string[] = [];
  if (d.files.length) parts.push(`${d.files.length} uncommitted file${d.files.length === 1 ? "" : "s"}`);
  if (d.main_changes?.files) parts.push(`${d.main_changes.files} file${d.main_changes.files === 1 ? "" : "s"} differ from main${d.main_changes.additions != null ? ` (+${d.main_changes.additions} −${d.main_changes.deletions ?? 0})` : ""}`);
  if (d.unpushed_commits) parts.push(`${d.unpushed_commits} unpushed commit${d.unpushed_commits === 1 ? "" : "s"}`);
  if (!parts.length) parts.push("No changes in Git");
  return <div className="flex min-w-0 items-start gap-1.5 text-[12.5px] text-ink-2"><Icon name="branch" size={13} className="mt-0.5 flex-none text-ink-3" /><div className="min-w-0"><div className="font-mono">{d.branch}</div><div>{parts.join(" · ")}</div></div></div>;
}

const OutcomeCard = memo(function OutcomeCard({ s, project }: { s: Session; project: string }) {
  const [busy, setBusy] = useState(false);
  const act = async (path: string, body?: unknown) => {
    setBusy(true);
    try { await api.post(`/api/sessions/${s.id}/${path}`, body); } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  const done = isDone(s);
  return <AgentCard s={s} className={done ? "border-line" : "border-gh-closed/40"}>
    <CardHead s={s} project={project} />
    {s.last_message && <p className="line-clamp-4 text-[13.5px] text-ink-2">{s.last_message}</p>}
    {done && <GitFacts id={s.id} />}
    <div className="pointer-events-none relative z-20 flex flex-wrap gap-2 [&_button]:pointer-events-auto">
      {done && <button className="btn" disabled={busy} title="Archive the session (it stays resumable)" onClick={() => void act("close")}><Icon name="archive" size={14} />Archive</button>}
      {!done && <button className="btn" disabled={busy} title="Send “Continue the unfinished turn.”" onClick={() => void act("input", { text: "Continue the unfinished turn." })}>Resume</button>}
    </div>
  </AgentCard>;
});

const PausedCard = memo(function PausedCard({ s, project }: { s: Session; project: string }) {
  const blockers = s.quota_pause?.blockers.map((b) => `${b.window} resets ${b.resets_at === null ? "at an unknown time" : new Date(b.resets_at * 1000).toLocaleString([], { weekday: "short", hour: "2-digit", minute: "2-digit" })}`).join(" · ");
  return <AgentCard s={s} className="phone-review-card border-attn-line">
    <CardHead s={s} project={project} />
    <p className="text-[13.5px] text-attn">{s.quota_pause?.error ?? blockers ?? "Waiting for a quota update"}</p>
    <div className="pointer-events-none relative z-20 flex flex-wrap gap-2 [&_button]:pointer-events-auto">
      <button className="btn" onClick={() => void api.post(`/api/sessions/${s.id}/interrupt`).catch(toastError)}><Icon name="stop" size={14} />Stop</button>
    </div>
  </AgentCard>;
});

function Row({ s, project }: { s: Session; project: string }) {
  return <Link to="/agents" search={{ session: s.id }} className="flex min-h-12 items-center gap-2.5 rounded-lg px-2 text-ink no-underline active:bg-sunk">
    <HarnessMark harness={s.harness} size={16} />
    <span className="min-w-0 flex-1"><span className="block truncate font-medium">{s.title}</span><span className="block truncate text-[12.5px] text-ink-3">{project}{s.last_message ? ` · ${s.last_message}` : ""}</span></span>
    <span className="lp-spin flex-none" title="Running" />
  </Link>;
}

/** "Updated just now" while connected, else the time of the last update. */
function Freshness() {
  const { online, since } = useConnection();
  const [, tick] = useState(0);
  useEffect(() => { const t = setInterval(() => tick((n) => n + 1), 20_000); return () => clearInterval(t); }, []);
  const saved = cachedAt();
  const clock = (ms: number) => new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  if (saved) return <p className="py-4 text-center text-[12.5px] text-attn" role="status">Saved data from {clock(saved)} · connecting…</p>;
  return <p className="py-4 text-center text-[12.5px] text-ink-3" role="status">{online ? "Live · updated just now" : `Offline · last update ${since ? new Date(since).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }) : "unknown"}`}</p>;
}

export function NowView() {
  const scope = useScope();
  const query = useSessions();
  const projects = useProjects().data;
  const sessions = (query.data ?? []).filter(scope.hasSession);
  const publish = usePublishItems();
  const name = (s: Session) => projectName(projects, s.project_id, s.cwd);
  const rank = (s: Session) => (s.open_question ? 0 : isDone(s) ? 1 : 2);
  const needs = sessions.filter(needsYou).sort((a, b) => rank(a) - rank(b) || b.status_since - a.status_since);
  const paused = sessions.filter(isQuotaPaused).sort((a, b) => b.status_since - a.status_since);
  const running = sessions.filter(isRunning).sort((a, b) => b.status_since - a.status_since);
  // Warm the session view of the first cards, so opening one shows its events at once.
  const load = useSessionLogs((st) => st.load);
  useEffect(() => {
    for (const s of needs.slice(0, 3)) {
      if (useSessionLogs.getState().logs[s.id]) continue;
      useSessionLogs.setState((st) => ({ logs: { ...st.logs, [s.id]: emptyLog() } }));
      void load(s.id);
    }
  }, [needs.map((s) => s.id).join(), load]);
  if (query.isPending) return <div className="grid h-full place-items-center text-ink-3" role="status">Loading…</div>;
  return (
    <div className="h-full overflow-y-auto [overflow-wrap:anywhere]">
      <div className="mx-auto grid min-w-0 max-w-[640px] grid-cols-1 gap-5 px-3 pt-4">
        <div className="flex min-w-0 items-center gap-2"><h1 className="min-w-0 flex-1 px-1 text-[19px] font-semibold">{needs.length ? `${needs.length} ${needs.length === 1 ? "agent needs" : "agents need"} you` : "Nothing waits for you"}</h1><NotificationsPrompt /></div>
        <OpenTerminals />
        {needs.length > 0 && <Section title="Needs you" count={needs.length}>
          {needs.map((s) => s.open_question ? <QuestionCard key={s.id} s={s} project={name(s)} /> : <OutcomeCard key={s.id} s={s} project={name(s)} />)}
        </Section>}
        {paused.length > 0 && <Section title="Quota paused" count={paused.length}>{paused.map((s) => <PausedCard key={s.id} s={s} project={name(s)} />)}</Section>}
        {publish.length > 0 && <Section title="Ready to publish" count={publish.length}>
          <Link to="/review" className="flex min-h-12 items-center gap-2 phone-review-card rounded-xl border border-line bg-paper px-3 text-ink no-underline"><Icon name="pr" size={16} />{publish.length} {publish.length === 1 ? "item waits" : "items wait"} for your review<Icon name="chev" size={14} className="ml-auto -rotate-90" /></Link>
        </Section>}
        <Section title="Running" count={running.length}>
          {running.length ? <div className="phone-review-card rounded-xl border border-line bg-paper p-1">{running.map((s) => <Row key={s.id} s={s} project={name(s)} />)}</div> : <p className="px-1 text-ink-3">No agent is running.</p>}
        </Section>
        <Freshness />
      </div>
    </div>
  );
}
