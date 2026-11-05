// Ask Launchpad as the built-in bot: the side panels of the Ask page, like a bot's page. Routines (the morning
// brief and others, posting to this workspace's thread), Instructions (its BOT.md with versions, and the
// built-in rules it cannot change), Threads, and Settings (profile, model, proposal mode, Reset to default).

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import { HarnessMark } from "@/components/HarnessMark";
import { Icon } from "@/components/Icon";
import { toast, toastError } from "@/components/Toast";
import { Badge, ConfirmButton, Loading } from "@/components/ui";
import { FileEditor, Section, Version } from "@/features/bots/BotsView";
import { NewRoutine, RoutineDetails, RunStrip } from "@/features/schedules/Routines";
import type { AskBot } from "@/gen/AskBot";
import type { Change } from "@/gen/Change";
import type { CronAgent } from "@/gen/CronAgent";
import type { Session } from "@/gen/Session";
import { api } from "@/lib/api";
import { ASK_BOT_ID, useAskBot } from "@/lib/bots";
import { formatAge } from "@/lib/format";
import { useProfiles } from "@/lib/profiles";
import { useConfig, useHarnessDefaults, useProjects, type ConfigResponse } from "@/lib/queries";
import { routineState, triggerSummary } from "@/lib/routines";
import { ProposalCard, useProposalList } from "./ProposalCard";

const BASE = "/api/assistant/bot";

/** Routines of Ask Launchpad in this workspace (`null`: Everything). */
export const askRoutines = (crons: CronAgent[], workspace: string | null) =>
  crons.filter((c) => !c.bot_id && c.spec.target.kind === "assistant" && ((c.spec.target as { workspace_id: string | null }).workspace_id ?? null) === workspace);

export function AskRoutinesPanel({ routines, workspace }: { routines: CronAgent[]; workspace: string | null }) {
  const projects = useProjects().data ?? [];
  const navigate = useNavigate();
  const [adding, setAdding] = useState(false);
  const [open, setOpen] = useState<string | null>(routines.length === 1 ? routines[0].id : null);
  return <div data-tour="ask.routines">
    <Section title="Routines" action={!adding && routines.length > 0 && <button className="btn btn-sm" onClick={() => setAdding(true)}><Icon name="plus" size={12} />New routine</button>}>
      <p className="m-0 text-[12px] text-ink-2">A routine sends Ask Launchpad a task by itself, in this workspace's thread: the morning brief, or a check when an agent is done. It answers with a short report or proposals, or with no update, which never notifies you.</p>
      {adding && <NewRoutine bot={null} workspaceId={workspace} repo={null} harness={null} onDone={() => setAdding(false)} />}
      {!adding && routines.length === 0 && <button className="btn btn-sm btn-primary w-fit" onClick={() => setAdding(true)}><Icon name="plus" size={12} />Add a routine</button>}
    </Section>
    {routines.map((r) => <Section key={r.id} title={<button className="flex w-full min-w-0 items-center gap-2 text-left normal-case" aria-expanded={open === r.id} onClick={() => setOpen(open === r.id ? null : r.id)}><span className="min-w-0 flex-1 truncate">{r.title}</span><RunStrip history={r.history} count={10} /></button>}>
      {open === r.id ? <RoutineDetails cron={r} onOpen={(sid) => void navigate({ to: "/agents", search: { session: sid } as never })} onDeleted={() => setOpen(null)} />
        : <p className="m-0 text-[12px] text-ink-3">{triggerSummary(r, (fid) => projects.find((p) => p.id === fid)?.name ?? "a removed folder")} · {routineState(r, "Ask Launchpad")}</p>}
    </Section>)}
  </div>;
}

export function AskInstructionsPanel() {
  const qc = useQueryClient();
  const bot = useAskBot();
  const history = useQuery({ queryKey: ["ask-bot-history", bot.data?.version], queryFn: () => api.get<Change[]>(`${BASE}/history`), enabled: !!bot.data });
  const proposed = (useProposalList().data ?? []).filter((p) => p.body.kind === "bot_instructions" && p.body.bot_id === ASK_BOT_ID && (p.state === "proposed" || p.state === "failed"));
  const save = async (instructions: string) => {
    try { qc.setQueryData(["ask-bot"], await api.patch<AskBot>(BASE, { instructions })); toast("Saved: new threads use it"); return true; } catch (e) { toastError(e); return false; }
  };
  if (bot.isLoading) return <Loading what="the instructions" />;
  if (!bot.data) return <p className="m-3 text-[12px] text-ink-3">The instructions are stored by Home: {bot.error ? String(bot.error) : "unavailable"}.</p>;
  const edited = bot.data.instructions !== bot.data.default_instructions;
  return <div data-tour="ask.instructions">
    {proposed.length > 0 && <Section title="Proposed by Ask Launchpad">{proposed.map((p) => <ProposalCard key={p.id} p={p} />)}</Section>}
    <Section title={<span className="flex items-center gap-2">BOT.md · how it works {edited ? <Badge tone="teal">Edited</Badge> : <Badge>Default</Badge>}</span>}>
      <p className="m-0 text-[12px] text-ink-2">How you want Ask Launchpad to work: what to check, what to propose, how to answer. New threads read it; the current thread keeps its own. You can also tell it “from now on, …”: it proposes the change here.</p>
      <FileEditor key={`i-${bot.data.version}`} value={bot.data.instructions} onSave={save} rows={14} tall />
    </Section>
    <Section title="Built in · not editable">
      <p className="m-0 text-[12px] text-ink-2">Launchpad adds its rules around BOT.md, and they win: Ask Launchpad only reads state and writes proposals, never starts, sends, publishes or edits anything itself, and never uses paid usage.</p>
      <details className="text-[12px]">
        <summary className="cursor-pointer text-ink-3">Show the built-in part</summary>
        <pre className="mt-1 max-h-[320px] overflow-auto rounded-md bg-sunk p-2 text-[11px] leading-4 whitespace-pre-wrap">{bot.data.builtin}</pre>
      </details>
    </Section>
    <Section title="Versions · last 30 days">
      {history.isLoading ? <Loading what="versions" /> : (history.data ?? []).length === 0 ? <p className="m-0 text-[12px] text-ink-3">No change yet: Ask Launchpad uses the default.</p>
        : (history.data ?? []).map((c) => <Version key={c.id} base={BASE} change={c} />)}
    </Section>
  </div>;
}

export function AskThreadsPanel({ threads }: { threads: Session[] }) {
  return <div data-tour="ask.threads">
    <Section title="One thread per workspace">
      <p className="m-0 text-[12px] text-ink-2">Your messages go to the current thread. New thread archives it; an archived thread stays resumable in Agents.</p>
    </Section>
    <Section title={`Threads · ${threads.length}`}>
      {threads.length === 0 ? <p className="m-0 text-[12px] text-ink-3">No thread yet in this workspace.</p>
        : threads.map((s) => <Link key={s.id} to="/agents" search={{ session: s.id } as never} className="grid gap-0.5 rounded-md px-2 py-1.5 no-underline hover:bg-sunk">
          <span className="flex items-center gap-2 text-[13px] text-ink"><span className="min-w-0 flex-1 truncate">{s.title}</span>{s.status !== "closed" && <Badge tone="teal">Current</Badge>}<HarnessMark harness={s.harness} size={12} /></span>
          <span className="truncate text-[12px] text-ink-3">{s.status === "closed" ? "archived" : s.status} · started {formatAge(s.started_at)} ago</span>
        </Link>)}
    </Section>
  </div>;
}

type Draft = { profile: string; model: string; auto: boolean };

export function AskSettingsPanel() {
  const qc = useQueryClient();
  const config = useConfig().data?.config.assistant;
  const bot = useAskBot();
  const profiles = useProfiles();
  const saved: Draft = { profile: config?.profile ?? "", model: config?.model ?? "", auto: config?.auto_mode ?? false };
  const [d, setD] = useState<Draft>(saved);
  const key = JSON.stringify(saved);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(() => setD(saved), [key]);
  const fallback = profiles.find((p) => p.billing === "subscription") ?? profiles[0];
  const profile = profiles.find((p) => p.id === d.profile) ?? fallback;
  const models = useHarnessDefaults(profile && !profile.models_url ? profile.harness : null).data?.models ?? [];
  const dirty = JSON.stringify(d) !== key;
  const save = async () => {
    try {
      qc.setQueryData(["config"], await api.patch<ConfigResponse>("/api/config", { assistant_profile: d.profile, assistant_model: d.model, ...(d.profile !== saved.profile || d.model !== saved.model ? { assistant_effort: "" } : {}), assistant_auto_mode: d.auto }));
      await qc.invalidateQueries({ queryKey: ["ask-bot"] });
      toast("Saved: new threads use it");
    } catch (e) { toastError(e); }
  };
  const reset = async () => {
    try {
      qc.setQueryData(["ask-bot"], await api.post<AskBot>(`${BASE}/reset`));
      await qc.invalidateQueries({ queryKey: ["config"] });
      toast("Ask Launchpad is back to its defaults");
    } catch (e) { toastError(e); }
  };
  return <div data-tour="ask.settings">
    <Section title="Settings">
      <label className="grid gap-1 text-[12px] font-semibold text-ink-2">Profile
        <select className="field-input h-[30px] font-normal" value={d.profile} onChange={(e) => setD({ ...d, profile: e.target.value, model: "" })}>
          <option value="">Default{fallback ? `: ${fallback.name}` : ""} (first subscription profile)</option>
          {profiles.map((p) => <option key={p.id} value={p.id}>{p.name} · {p.billing === "subscription" ? "subscription" : "pay as you go"}</option>)}
        </select>
      </label>
      <label className="grid gap-1 text-[12px] font-semibold text-ink-2">Model
        {models.length || !profile?.models_url
          ? <select className="field-input h-[30px] font-normal" value={d.model} onChange={(e) => setD({ ...d, model: e.target.value })}>
            <option value="">Default of the profile{profile?.model ? ` (${profile.model})` : ""}</option>
            {models.map((m) => <option key={m.id} value={m.id}>{m.display_name || m.id}</option>)}
            {d.model && !models.some((m) => m.id === d.model) && <option value={d.model}>{d.model}</option>}
          </select>
          : <input className="field-input h-[30px] font-mono text-[12px] font-normal" value={d.model} placeholder={profile?.model ?? "profile default"} onChange={(e) => setD({ ...d, model: e.target.value.trim() })} />}
        <span className="font-normal text-ink-3">A fast model keeps replies quick. Changing the model of a thread (Summary → Run) also saves it here.</span>
      </label>
      <fieldset className="m-0 grid gap-1 border-0 p-0">
        <legend className="pb-1 text-[12px] font-semibold text-ink-2">Proposal mode</legend>
        <label className="flex min-h-[28px] items-start gap-2 text-[13px]"><input type="radio" className="mt-1" name="ask-mode" checked={!d.auto} onChange={() => setD({ ...d, auto: false })} /><span><span className="font-semibold">Manual</span> <span className="text-ink-3">· every proposal waits for your click</span></span></label>
        <label className="flex min-h-[28px] items-start gap-2 text-[13px]"><input type="radio" className="mt-1" name="ask-mode" checked={d.auto} onChange={() => setD({ ...d, auto: true })} /><span><span className="font-semibold">Auto</span> <span className="text-ink-3">· new launches, messages, schedules and presets apply at once; routines, bots, tools and BOT.md changes still wait</span></span></label>
      </fieldset>
      {dirty && <div className="flex justify-end gap-1.5"><button className="btn btn-sm btn-ghost" onClick={() => setD(saved)}>Discard</button><button className="btn btn-sm btn-primary" onClick={() => void save()}>Save</button></div>}
    </Section>
    <Section title="What it can reach">
      <ul className="m-0 grid gap-1 pl-4 text-[12px] text-ink-2">
        <li>Reads Launchpad state with a restricted token: folders, agents and their traces, bots, Inbox, schedules, quotas and proposals.</li>
        <li>Writes proposals only. It never starts agents, sends messages, schedules, publishes or posts to GitHub by itself.</li>
        <li>No folder, worktree or tool; paid usage is always off.</li>
      </ul>
    </Section>
    <Section title="Reset">
      <p className="m-0 text-[12px] text-ink-3">Puts back the default BOT.md and settings: the first subscription profile, its default model and Manual mode. Threads, proposals and routines stay, and Restore previous in Instructions brings your BOT.md back.</p>
      <ConfirmButton icon="refresh" className="w-fit" disabled={!bot.data?.customized} title={bot.data?.customized ? undefined : "Ask Launchpad already uses its defaults"} consequence="Reset Ask Launchpad's instructions and settings to their defaults? New threads use them; the current thread keeps its own." confirmLabel="Reset to default" onConfirm={reset}>Reset to default</ConfirmButton>
    </Section>
  </div>;
}
