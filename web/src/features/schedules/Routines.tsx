// Routines: the strip of recent runs, the trigger form shared by bots and the Schedules page, a routine's
// details (state, runs, Run now, Pause, Edit, Delete) and the form that adds a routine to a bot.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { memo, useEffect, useState } from "react";
import { useHarnessName } from "@/components/HarnessMark";
import { Icon } from "@/components/Icon";
import { toast, toastError } from "@/components/Toast";
import { Badge, ConfirmButton, FilterChip } from "@/components/ui";
import { CronFields, DEFAULT_CRON, conditionSummary } from "@/features/launch/CronSelector";
import type { Bot } from "@/gen/Bot";
import type { CronAgent } from "@/gen/CronAgent";
import type { CronCondition } from "@/gen/CronCondition";
import type { GithubEvent } from "@/gen/GithubEvent";
import type { HarnessKind } from "@/gen/HarnessKind";
import type { Preview } from "@/gen/Preview";
import type { Project } from "@/gen/Project";
import type { RoutineRun } from "@/gen/RoutineRun";
import type { Session } from "@/gen/Session";
import type { Trigger } from "@/gen/Trigger";
import { api } from "@/lib/api";
import { formatAge } from "@/lib/format";
import { useProjects } from "@/lib/queries";
import { GITHUB_EVENTS, OUTCOME, OUTCOME_CLASS, browserZone, lastSuccess, routineState, triggerSummary } from "@/lib/routines";

export type RoutineDraft = { trigger: Trigger; schedule: string; conditions: CronCondition[] };

const TONE = { ok: "ok", quiet: "neutral", bad: "danger", work: "work" } as const;

/** The last runs as small squares, oldest first: report, no update, skipped, failed. */
export const RunStrip = memo(function RunStrip({ history, count = 14 }: { history: RoutineRun[]; count?: number }) {
  const runs = history.slice(-count);
  if (!runs.length) return <span className="text-[11px] text-ink-3">No run yet</span>;
  return <span className="flex items-center gap-[3px]" aria-label="Recent runs">
    {runs.map((r) => <span key={`${r.at}-${r.cause}`} className={`size-[9px] rounded-[2px] ${OUTCOME_CLASS[r.outcome]}`} title={`${new Date(r.at).toLocaleString()} · ${OUTCOME[r.outcome].label}${r.detail ? `: ${r.detail}` : ""} · ${r.cause}`} />)}
  </span>;
});

export function OutcomeBadge({ run }: { run: RoutineRun }) {
  const o = OUTCOME[run.outcome];
  return <Badge tone={TONE[o.tone]}>{run.outcome === "running" && <span className="lp-spin mr-1" />}{o.label}</Badge>;
}

/** The repositories of a workspace's folders. */
function repositories(projects: Project[], workspaceId: string | null): string[] {
  return [...new Set(projects.filter((p) => p.workspace_id === workspaceId).flatMap((p) => [p.gh_repo, p.gh_upstream]).filter((r): r is string => !!r))].sort();
}

/** When a routine runs: a schedule, GitHub activity or another agent's turn, and its conditions. */
export function TriggerFields({ draft, harness, workspaceId, inThread, onChange, onValidityChange }: { draft: RoutineDraft; harness: HarnessKind | null; workspaceId: string | null; inThread: boolean; onChange: (d: RoutineDraft) => void; onValidityChange: (valid: boolean) => void }) {
  const projects = useProjects().data ?? [];
  const repos = repositories(projects, workspaceId);
  const folders = projects.filter((p) => p.workspace_id === workspaceId && !p.is_temp).sort((a, b) => a.name.localeCompare(b.name));
  const [fieldsValid, setFieldsValid] = useState(true);
  const t = draft.trigger;
  const valid = fieldsValid && (t.kind !== "github" || t.events.length > 0);
  useEffect(() => onValidityChange(valid), [valid, onValidityChange]);
  const kind = (k: Trigger["kind"]) => onChange({ ...draft, trigger: k === "schedule" ? { kind: "schedule" } : k === "github" ? { kind: "github", repo: null, events: ["review_requested"] } : { kind: "session_done", folder: null }, schedule: draft.schedule || DEFAULT_CRON.schedule });
  const toggle = (e: GithubEvent) => t.kind === "github" && onChange({ ...draft, trigger: { ...t, events: t.events.includes(e) ? t.events.filter((x) => x !== e) : [...t.events, e] } });
  // In a thread (a bot, Ask Launchpad), a run waits for the thread anyway: no "previous run" condition.
  const conditions = inThread ? draft.conditions.filter((c) => c.kind !== "previous_run_finished") : draft.conditions;
  return <div className="grid gap-2">
    <div className="flex flex-wrap gap-1.5" role="radiogroup" aria-label="When it runs">
      <FilterChip active={t.kind === "schedule"} onClick={() => kind("schedule")}>On a schedule</FilterChip>
      <FilterChip active={t.kind === "github"} onClick={() => kind("github")}>GitHub activity</FilterChip>
      <FilterChip active={t.kind === "session_done"} onClick={() => kind("session_done")}>When an agent is done</FilterChip>
    </div>
    {t.kind === "github" && <div className="grid gap-2">
      <label className="grid gap-1 text-[12px] font-semibold text-ink-2">Repository
        <select className="field-input h-[30px] font-normal" value={t.repo ?? ""} onChange={(e) => onChange({ ...draft, trigger: { ...t, repo: e.target.value || null } })}>
          <option value="">Any repository of the workspace{repos.length ? ` (${repos.length})` : ""}</option>
          {repos.map((r) => <option key={r} value={r}>{r}</option>)}
          {t.repo && !repos.includes(t.repo) && <option value={t.repo}>{t.repo}</option>}
        </select>
      </label>
      <div className="grid gap-1"><span className="text-[12px] font-semibold text-ink-2">New in your Inbox</span>
        <div className="flex flex-wrap gap-1.5">{GITHUB_EVENTS.map((e) => <FilterChip key={e.id} active={t.events.includes(e.id)} onClick={() => toggle(e.id)}>{e.label}</FilterChip>)}</div>
        {!t.events.length && <p role="alert" className="m-0 text-xs text-gh-closed">Pick at least one kind of activity.</p>}
      </div>
      <p className="m-0 text-xs text-ink-3">Launchpad reads your GitHub notifications (the Inbox) about every minute. New activity found in one check starts one run with all of it.</p>
    </div>}
    {t.kind === "session_done" && <div className="grid gap-2">
      <label className="grid gap-1 text-[12px] font-semibold text-ink-2">Folder
        <select className="field-input h-[30px] font-normal" value={t.folder ?? ""} onChange={(e) => onChange({ ...draft, trigger: { ...t, folder: e.target.value || null } })}>
          <option value="">Any folder of the workspace</option>
          {folders.map((p) => <option key={p.id} value={p.id}>{p.name}</option>)}
        </select>
      </label>
      <p className="m-0 text-xs text-ink-3">Each time another agent finishes a turn (Done) there, a run starts with a reference to that agent's trace. Runs of routines never start other routines.</p>
    </div>}
    <CronFields draft={{ schedule: draft.schedule, conditions }} harness={harness} schedule={t.kind === "schedule"} previousRun={!inThread} onChange={(c) => onChange({ ...draft, schedule: c.schedule, conditions: c.conditions })} onValidityChange={setFieldsValid} />
  </div>;
}

/** A routine's state, recent runs, actions and history. Used in the Schedules page and on a bot's page. */
export function RoutineDetails({ cron, bot, onOpen, onDeleted }: { cron: CronAgent; bot?: Bot | null; onOpen: (sessionId: string) => void; onDeleted?: () => void }) {
  const qc = useQueryClient();
  const harnessName = useHarnessName();
  const projects = useProjects().data ?? [];
  const [busy, setBusy] = useState(false);
  const [edit, setEdit] = useState<(RoutineDraft & { title: string; instructions: string }) | null>(null);
  const [editValid, setEditValid] = useState(true);
  const inThread = !!cron.bot_id || cron.spec.target.kind === "assistant";
  const preview = useQuery({ queryKey: ["cron-preview", cron.id], queryFn: () => api.post<Preview>("/api/launch/preview", cron.spec), enabled: !inThread });
  const projectId = cron.spec.target.kind === "project" || cron.spec.target.kind === "predraft" ? cron.spec.target.project_id : null;
  const folder = projects.find((p) => p.id === projectId)?.name;
  const success = lastSuccess(cron);
  const act = async (f: () => Promise<void>) => {
    setBusy(true);
    try { await f(); await qc.invalidateQueries({ queryKey: ["crons"] }); } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  const save = () => act(async () => {
    if (!edit) return;
    await api.patch(`/api/launch/crons/${cron.id}`, { title: edit.title, trigger: edit.trigger, schedule: edit.schedule, conditions: edit.conditions, time_zone: browserZone(), ...(cron.bot_id ? { instructions: edit.instructions } : {}) });
    setEdit(null);
    toast("Routine saved");
  });
  const where = cron.bot_id ? <>Posts to {bot ? <Link to="/bots/$id" params={{ id: bot.id }}>{bot.settings.name}</Link> : "its bot"}'s thread</> : cron.spec.target.kind === "assistant" ? <>Posts to <Link to="/ask">Ask Launchpad</Link></> : <>Starts a new agent · {cron.spec.harness ? harnessName(cron.spec.harness) : "Agent"} · {folder ?? "a new scratch folder"}</>;
  return <div className="grid min-w-0 grid-cols-[minmax(0,1fr)] gap-3 text-[13px]">
    <div className="grid min-w-0 grid-cols-[minmax(0,1fr)] gap-2 rounded-lg border border-line bg-chrome p-3">
      {edit ? <>
        <label className="grid gap-1 text-[12px] font-semibold text-ink-2">Name<input className="field-input h-[30px] font-normal" value={edit.title} maxLength={80} onChange={(e) => setEdit({ ...edit, title: e.target.value })} /></label>
        {cron.bot_id && <label className="grid gap-1 text-[12px] font-semibold text-ink-2">What it does at each run<textarea className="field-input min-h-[90px] py-1.5 font-normal leading-5" value={edit.instructions} onChange={(e) => setEdit({ ...edit, instructions: e.target.value })} /></label>}
        <TriggerFields draft={edit} harness={cron.spec.harness} workspaceId={cron.spec.workspace_id} inThread={inThread} onChange={(d) => setEdit({ ...edit, ...d })} onValidityChange={setEditValid} />
        <div className="flex justify-end gap-2">
          <button className="btn btn-sm btn-ghost" disabled={busy} onClick={() => setEdit(null)}>Cancel</button>
          <button className="btn btn-sm btn-primary" disabled={busy || !editValid || !edit.title.trim() || (!!cron.bot_id && !edit.instructions.trim())} onClick={() => void save()}>Save</button>
        </div>
      </> : <>
        <div className="flex flex-wrap items-center gap-2">
          <b>{routineState(cron, bot?.settings.name)}</b>
          <span className="ml-auto"><RunStrip history={cron.history} /></span>
        </div>
        <p className="m-0 text-ink-2">{triggerSummary(cron, (id) => projects.find((p) => p.id === id)?.name ?? "a removed folder")}</p>
        <p className="m-0 text-[12px] text-ink-3">{where}</p>
        {cron.conditions.length > 0 && <ul className="m-0 list-disc pl-5 text-xs text-ink-2">{cron.conditions.map((c, i) => <li key={i}>{conditionSummary(c, cron.spec.harness, harnessName)}</li>)}</ul>}
        <p className="m-0 text-xs text-ink-3">{success ? `Last successful run ${formatAge(success.at)} ago (${success.outcome === "report" ? "report" : "nothing to report"}).` : "No successful run yet."}{cron.last_outcome && !cron.last_outcome.startsWith("Report") && !cron.last_outcome.startsWith("No update") ? ` Last: ${cron.last_outcome}.` : ""}</p>
        <div className="flex flex-wrap gap-2">
          <button className="btn btn-sm btn-primary" disabled={busy} title="Starts a run now, whatever the trigger and the conditions" onClick={() => void act(async () => { onOpen((await api.post<Session>(`/api/launch/crons/${cron.id}/run`)).id); })}><Icon name="play" size={12} />Run now</button>
          <button className="btn btn-sm" disabled={busy} onClick={() => setEdit({ trigger: cron.trigger, schedule: cron.schedule, conditions: cron.conditions, title: cron.title, instructions: cron.spec.text })}><Icon name="pencil" size={12} />Edit</button>
          <button className="btn btn-sm" disabled={busy || cron.bot_paused} title={cron.bot_paused ? "Resume the bot to resume its routines" : cron.paused ? "Runs start again from the next trigger" : "No run starts until you resume it"} onClick={() => void act(async () => { await api.patch(`/api/launch/crons/${cron.id}`, { paused: !cron.paused }); })}>{cron.paused ? "Resume" : "Pause"}</button>
          <ConfirmButton icon="trash" consequence={`Delete the routine “${cron.title}”? Its past runs stay in Agents.`} confirmLabel="Delete routine" onConfirm={() => act(async () => { await api.delete(`/api/launch/crons/${cron.id}`); onDeleted?.(); })}>Delete</ConfirmButton>
        </div>
      </>}
    </div>
    {cron.bot_id && !edit && <div className="grid gap-1"><b className="text-xs text-ink-2">What it does at each run</b><p className="m-0 rounded-md bg-sunk px-3 py-2 text-[12px] break-words whitespace-pre-wrap text-ink-2">{cron.spec.text}</p></div>}
    <div className="grid min-w-0 grid-cols-[minmax(0,1fr)] gap-1">
      <b className="text-xs text-ink-2">Runs ({cron.history.length})</b>
      {cron.history.length ? [...cron.history].reverse().map((r) => <button key={`${r.at}-${r.cause}`} disabled={!r.session_id} onClick={() => r.session_id && onOpen(r.session_id)} className="grid min-w-0 gap-0.5 rounded-md border border-line px-2 py-1.5 text-left text-xs hover:bg-chrome disabled:cursor-default disabled:hover:bg-transparent">
        <span className="flex min-w-0 items-center gap-2"><span className="min-w-0 flex-1 truncate" title={r.cause}>{new Date(r.at).toLocaleString([], { dateStyle: "medium", timeStyle: "short" })} · {r.cause}</span><OutcomeBadge run={r} /></span>
        {r.detail && <span className="text-ink-3">{r.detail}</span>}
      </button>) : <p className="m-0 text-xs text-ink-3">No run yet. Quiet runs show here as No update; they never notify you.</p>}
    </div>
    {!inThread && <div><b className="text-xs text-ink-2">Prompt of each run</b><pre className="mt-2 rounded-lg border border-line p-3 text-xs whitespace-pre-wrap">{preview.data?.prompt ?? (preview.isError ? "Folder context is unavailable. Delete this routine and create it again with another folder." : "Loading…")}</pre></div>}
  </div>;
}

/** Ready-made routines for a bot, from its folder. Each fills the form; nothing is saved until Add routine. */
export function routineTemplates(repo: string | null): { label: string; title: string; instructions: string; draft: RoutineDraft }[] {
  const cond = DEFAULT_CRON.conditions.filter((c) => c.kind === "quota_left");
  return [
    { label: "Daily check", title: "Daily check", instructions: "Look at what changed in your folder and in the knowledge base since your last run. Report only what needs my attention, in a few lines.", draft: { trigger: { kind: "schedule" }, schedule: DEFAULT_CRON.schedule, conditions: cond } },
    ...(repo ? [
      { label: "Review requests", title: "Review requests", instructions: "Review each pull request below. Prepare your review as a suggestion (launchpad suggest review) for me to post; do not post it yourself. Report one line per pull request with your verdict.", draft: { trigger: { kind: "github" as const, repo, events: ["review_requested" as const] }, schedule: "", conditions: cond } },
      { label: "CI failures", title: "CI failures", instructions: "Read the failing logs below. If the change itself causes the failure, prepare a fix in a new worktree as a Local PR and report. If it is flaky or unrelated, reply no_update.", draft: { trigger: { kind: "github" as const, repo, events: ["ci_activity" as const] }, schedule: "", conditions: cond } },
    ] : []),
    { label: "After each agent", title: "Review finished work", instructions: "Read what the agent below did. If something looks wrong or unfinished, report it with the file and line. Otherwise reply no_update.", draft: { trigger: { kind: "session_done", folder: null }, schedule: "", conditions: cond } },
  ];
}

/** Adds a routine to a bot: a name, what it does, and when. */
export function NewRoutine({ bot, repo, harness, onDone }: { bot: Bot; repo: string | null; harness: HarnessKind | null; onDone: () => void }) {
  const qc = useQueryClient();
  const [title, setTitle] = useState("");
  const [instructions, setInstructions] = useState("");
  const [draft, setDraft] = useState<RoutineDraft>({ trigger: { kind: "schedule" }, schedule: DEFAULT_CRON.schedule, conditions: DEFAULT_CRON.conditions.filter((c) => c.kind === "quota_left") });
  const [valid, setValid] = useState(true);
  const [busy, setBusy] = useState(false);
  const add = async () => {
    setBusy(true);
    try {
      await api.post<CronAgent>(`/api/bots/${bot.id}/routines`, { title: title.trim(), instructions: instructions.trim(), ...draft, time_zone: browserZone() });
      await qc.invalidateQueries({ queryKey: ["crons"] });
      toast(`Routine added to ${bot.settings.name}`);
      onDone();
    } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  return <div className="grid gap-2 rounded-lg border border-line bg-paper p-3" data-tour="bots.routine-form">
    {!title && !instructions && <div className="flex flex-wrap items-center gap-1.5 text-[12px] text-ink-3">Start from
      {routineTemplates(repo).map((t) => <button key={t.label} className="btn btn-sm" onClick={() => { setTitle(t.title); setInstructions(t.instructions); setDraft(t.draft); }}>{t.label}</button>)}
    </div>}
    <label className="grid gap-1 text-[12px] font-semibold text-ink-2">Name<input className="field-input h-[30px] font-normal" maxLength={80} value={title} placeholder="Morning check" onChange={(e) => setTitle(e.target.value)} /></label>
    <label className="grid gap-1 text-[12px] font-semibold text-ink-2">What it does at each run
      <textarea className="field-input min-h-[76px] py-1.5 font-normal leading-5" value={instructions} placeholder="For example: “Check the open issues labeled bug and tell me about new ones.”" onChange={(e) => setInstructions(e.target.value)} />
      <span className="font-normal text-ink-3">Sent to {bot.settings.name}'s thread at each run. It reports, or replies no_update when there is nothing to say: then you get no notification.</span>
    </label>
    <TriggerFields draft={draft} harness={harness} workspaceId={bot.workspace_id} inThread onChange={setDraft} onValidityChange={setValid} />
    <div className="flex justify-end gap-2">
      <button className="btn btn-sm btn-ghost" onClick={onDone}>Cancel</button>
      <button className="btn btn-sm btn-primary" disabled={busy || !valid || !title.trim() || !instructions.trim()} onClick={() => void add()}><Icon name="plus" size={12} />Add routine</button>
    </div>
  </div>;
}
