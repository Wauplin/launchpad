// Schedules: everything that starts on its own, in one place. Routines run again and again (on a schedule, on
// GitHub activity, when another agent is done); scheduled launches start once, when their conditions are met.

import { useQueryClient } from "@tanstack/react-query";
import { useNavigate, useSearch } from "@tanstack/react-router";
import { memo, type ReactNode } from "react";
import { Icon } from "@/components/Icon";
import { toast, toastError } from "@/components/Toast";
import { Empty, ErrorState, Loading, PageHeader, Tabs } from "@/components/ui";
import { useComposer } from "@/stores/composer";
import { conditionsSummary } from "@/features/launch/DelaySelector";
import { DEFAULT_CRON } from "@/features/launch/CronSelector";
import { ScheduledPane } from "@/features/agents/ScheduledPane";
import { BotAvatar } from "@/features/bots/BotsView";
import type { Bot } from "@/gen/Bot";
import type { CronAgent } from "@/gen/CronAgent";
import type { ScheduledLaunch } from "@/gen/ScheduledLaunch";
import { api } from "@/lib/api";
import { askTarget, useBots } from "@/lib/bots";
import { usePhone } from "@/lib/phone";
import { when } from "@/lib/format";
import { useCrons, useProjects, useSchedules, useSessions } from "@/lib/queries";
import { browserZone, cronTitle, routineState, triggerSummary } from "@/lib/routines";
import { useScope } from "@/lib/workspace";
import { PhoneBar, PhoneHeader } from "@/mobile/PhoneHeader";
import { BRIEF, BRIEF_TITLE, RoutineDetails, RunStrip } from "./Routines";

type Tab = "routines" | "launches";

/** The morning brief: a routine of a bot (the workspace's first, Ask Launchpad unless you changed it), weekdays at 8:30. */
export function useCreateBrief() {
  const qc = useQueryClient();
  return async (bot: Bot) => {
    try {
      const r = await api.post<CronAgent>(`/api/bots/${bot.id}/routines`, { title: BRIEF_TITLE, instructions: BRIEF, trigger: { kind: "schedule" }, schedule: "30 8 * * 1-5", conditions: DEFAULT_CRON.conditions.filter((c) => c.kind === "quota_left"), time_zone: browserZone() });
      await qc.invalidateQueries({ queryKey: ["crons"] });
      toast(`Morning brief added: weekdays at 8:30, by ${bot.settings.name}`);
      return r;
    } catch (e) { toastError(e); return null; }
  };
}

export function SchedulesView() {
  const phone = usePhone();
  const scope = useScope();
  const navigate = useNavigate();
  const search = useSearch({ strict: false }) as { routine?: string; launch?: string; tab?: Tab };
  const crons = useCrons();
  const schedules = useSchedules();
  const sessions = useSessions().data ?? [];
  const bots = useBots().data ?? [];
  const routines = (crons.data ?? []).filter((c) => scope.hasSpec(c.spec));
  const launches = (schedules.data ?? []).filter((l) => scope.hasSpec(l.spec));
  const tab: Tab = search.tab ?? (search.launch ? "launches" : "routines");
  const go = (s: { tab?: Tab; routine?: string; launch?: string }) => void navigate({ to: "/schedules", search: s as never });
  const selected = routines.find((r) => r.id === search.routine);
  const launch = launches.find((l) => l.id === search.launch);
  const openSession = (id: string) => void navigate({ to: "/agents", search: { session: id } as never });
  const createBrief = useCreateBrief();
  const hasBrief = routines.some((r) => !!r.bot_id && r.title === BRIEF_TITLE);
  const briefBot = askTarget(bots, scope.workspace.id);
  const next = routines.filter((r) => !r.paused && !r.bot_paused && r.next_run_at).reduce<number | null>((min, r) => min == null || r.next_run_at! < min ? r.next_run_at : min, null);
  const tabs = <Tabs label="Schedules" value={tab} onChange={(t) => go({ tab: t })} tour="schedules.tabs" items={[{ id: "routines", label: "Routines", count: routines.length || undefined }, { id: "launches", label: "Scheduled launches", count: launches.length || undefined }]} />;
  const header = phone
    ? <PhoneHeader title="Schedules" details={<p className="m-0 text-[13px] text-ink-2">{INTRO}</p>} />
    : <PageHeader title="Schedules" meta={next ? `Next run ${when(next)}` : undefined} intro={INTRO} tabs={tabs} />;
  const panel = selected
    ? <Side title={selected.title} onClose={() => go({ tab })}><RoutineDetails cron={selected} bot={bots.find((b) => b.id === selected.bot_id)} onOpen={openSession} onDeleted={() => go({ tab })} /></Side>
    : launch ? <ScheduledPane launch={launch} onClose={() => go({ tab })} onStart={(s) => openSession(s.id)} /> : null;
  const loading = crons.isLoading || schedules.isLoading;
  const error = crons.error ?? schedules.error;
  const list = <div className="mx-auto grid w-full max-w-[880px] grid-cols-[minmax(0,1fr)] gap-3 px-5 py-4" data-tour="schedules.page">
    {phone && tabs}
    {loading ? <Loading what="schedules" /> : error ? <ErrorState error={error} onRetry={() => { void crons.refetch(); void schedules.refetch(); }} /> : tab === "routines" ? <>
      {routines.length > 0 && <div className="grid grid-cols-[minmax(0,1fr)] overflow-hidden rounded-lg border border-line" data-tour="schedules.routines">
        {routines.map((r) => <RoutineRow key={r.id} r={r} bot={bots.find((b) => b.id === r.bot_id)} selected={r.id === selected?.id} onClick={() => go({ tab, routine: r.id })} />)}
      </div>}
      {!hasBrief && briefBot && <Idea icon="spark" title="Morning brief" body={`${briefBot.settings.name} summarizes what needs you, weekdays at 8:30. A quiet morning sends nothing.`} action={<button className="btn btn-sm btn-primary" data-tour="schedules.brief" onClick={async () => { const r = await createBrief(briefBot); if (r) go({ tab, routine: r.id }); }}><Icon name="plus" size={12} />Add the morning brief</button>} />}
    </> : launches.length ? <div className="grid grid-cols-[minmax(0,1fr)] overflow-hidden rounded-lg border border-line">
      {launches.map((l) => <LaunchRow key={l.id} l={l} detail={conditionsSummary(l, sessions, launches)} selected={l.id === launch?.id} onClick={() => go({ tab, launch: l.id })} />)}
    </div> : <Empty icon="clock" action={<OpenComposer />}>Nothing scheduled. In the composer, the arrow next to Launch → Start later starts an agent after a delay, at a time, after other agents, or when quota is back.</Empty>}
  </div>;
  if (phone)
    return <div className="flex h-full min-h-0 flex-col">
      {panel ? <div className="min-h-0 flex-1 overflow-y-auto">{panel}</div> : <>{header}<div className="min-h-0 flex-1 overflow-y-auto">{list}</div></>}
    </div>;
  return (
    <div className="flex h-full min-h-0 flex-col">
      {header}
      <div className={`grid min-h-0 flex-1 ${panel ? "grid-cols-[minmax(0,1fr)_440px]" : "grid-cols-[minmax(0,1fr)]"}`}>
        <div className="min-h-0 overflow-y-auto">{list}</div>
        {panel && <aside className="min-h-0 overflow-y-auto border-l border-line">{panel}</aside>}
      </div>
    </div>
  );
}

/** The page explanation (ⓘ): what routines and scheduled launches are, and where to create them. */
const INTRO = "Routines start on their own, on a schedule or an event; scheduled launches start once. Give a bot a routine from its Routines panel, or turn any prompt into a routine in the composer: the arrow next to Launch → Repeat on a schedule. Each run starts a new agent, or messages the bot's thread.";

function OpenComposer() {
  const start = useComposer((s) => s.start);
  return <button className="btn btn-sm" onClick={() => start()}><Icon name="play" size={12} />Open the composer</button>;
}

function Side({ title, onClose, children }: { title: string; onClose: () => void; children: ReactNode }) {
  const phone = usePhone();
  return <div className="flex h-full flex-col bg-paper">
    {phone ? <PhoneBar title={title} onBack={onClose} backLabel="Back to schedules" /> : <div className="flex items-center gap-2 border-b border-line px-4 py-3"><b className="min-w-0 flex-1 truncate text-sm">{title}</b><button className="btn btn-sm" onClick={onClose}>Close panel</button></div>}
    <div className="min-h-0 flex-1 overflow-y-auto p-4">{children}</div>
  </div>;
}

function Idea({ icon, title, body, action }: { icon: "spark"; title: string; body: string; action: ReactNode }) {
  return <section className="flex flex-wrap items-center gap-3 rounded-lg border border-dashed border-line px-3 py-2.5">
    <Icon name={icon} size={16} className="text-teal-ink" />
    <span className="min-w-0 flex-1 basis-[260px]"><b className="block text-[13px]">{title}</b><span className="text-[12px] text-ink-3">{body}</span></span>
    {action}
  </section>;
}

const RoutineRow = memo(function RoutineRow({ r, bot, selected, onClick }: { r: CronAgent; bot?: Bot; selected: boolean; onClick: () => void }) {
  const projects = useProjects().data ?? [];
  const owner = bot ? bot.settings.name : null;
  return <button onClick={onClick} className={`grid min-h-[60px] w-full grid-cols-[28px_minmax(0,1fr)_auto] items-center gap-x-3 border-b border-line px-3 py-2 text-left last:border-b-0 hover:bg-sunk ${selected ? "bg-select" : "bg-paper"}`}>
    {bot ? <BotAvatar name={bot.settings.name} size={28} /> : <span className="grid size-[28px] place-items-center rounded-[8px] bg-teal-soft text-teal-ink"><Icon name="refresh" size={14} /></span>}
    <span className="min-w-0">
      <span className="block truncate text-[13px] font-semibold">{r.title}{owner && <span className="font-normal text-ink-3"> · {owner}</span>}</span>
      <span className="block truncate text-[12px] text-ink-3" title={cronTitle(r)}>{triggerSummary(r, (id) => projects.find((p) => p.id === id)?.name ?? "a removed folder")} · {routineState(r, bot?.settings.name)}</span>
    </span>
    <RunStrip history={r.history} />
  </button>;
});

const LaunchRow = memo(function LaunchRow({ l, detail, selected, onClick }: { l: ScheduledLaunch; detail: string; selected: boolean; onClick: () => void }) {
  return <button onClick={onClick} className={`grid min-h-[52px] w-full grid-cols-[16px_minmax(0,1fr)] items-center gap-x-3 border-b border-line px-3 py-2 text-left last:border-b-0 hover:bg-sunk ${selected ? "bg-select" : "bg-paper"}`}>
    <Icon name="clock" size={14} className="text-ink-3" />
    <span className="min-w-0">
      <span className="block truncate text-[13px] font-semibold">{l.resume_session_id ? "Follow-up · " : ""}{l.title}</span>
      <span className="block truncate text-[12px] text-ink-3">{l.status === "failed" ? "Needs attention · " : l.status === "starting" ? "Starting · " : ""}{detail || "Waiting for conditions"}</span>
    </span>
  </button>;
});
