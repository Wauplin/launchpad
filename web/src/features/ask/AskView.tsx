// Ask Launchpad: a chat with the workspace's assistant session that turns requests into proposal cards.
// Cards are inert until the owner clicks Start (or Send, Schedule, Create cron); started cards become live
// status rows bound to their session through the event bus. The thread is a normal session (SessionPane).

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate, useSearch } from "@tanstack/react-router";
import { memo, useEffect, useRef, useState } from "react";
import type { Proposal } from "@/gen/Proposal";
import type { Session } from "@/gen/Session";
import type { LaunchSpec } from "@/gen/LaunchSpec";
import { HarnessMark } from "@/components/HarnessMark";
import { Icon } from "@/components/Icon";
import { MenuItem, Popover } from "@/components/Popover";
import { toast, toastError } from "@/components/Toast";
import { ConfirmButton, Empty, ErrorState, FilterChip, FilterChips, Loading, PageHeader, Tabs, Token } from "@/components/ui";
import { api } from "@/lib/api";
import { usePhone } from "@/lib/phone";
import { useProfiles } from "@/lib/profiles";
import { useConfig, useHarnessDefaults, useProjects, useSessions, type ConfigResponse } from "@/lib/queries";
import { needsYou, isQuotaPaused, isRunning } from "@/lib/sessionStatus";
import { useScope } from "@/lib/workspace";
import { useComposer } from "@/stores/composer";
import { useWorkspace } from "@/stores/workspace";
import { useLayout } from "@/stores/layout";
import { SessionPane } from "@/features/agents/SessionPane";
import { StatusBadge } from "@/features/agents/SessionBits";
import { PhoneActions } from "@/mobile/PhoneHeader";

const useProposalList = () => useQuery({ queryKey: ["proposals"], queryFn: () => api.get<Proposal[]>("/api/proposals"), staleTime: 30_000 });

const START: Record<Proposal["body"]["kind"], string> = { launch: "Start", message: "Send", schedule: "Schedule", cron: "Create cron", status: "" };

function specOf(p: Proposal): LaunchSpec | null {
  return p.body.kind === "launch" || p.body.kind === "schedule" || p.body.kind === "cron" ? p.body.spec : null;
}

/** Profile, then only values that differ from defaults, then warnings, like the composer's option line. */
function SpecTokens({ spec }: { spec: LaunchSpec }) {
  const profiles = useProfiles();
  const projects = useProjects().data ?? [];
  const profile = profiles.find((x) => x.id === spec.profile);
  const folder = spec.target.kind === "project" || spec.target.kind === "predraft" ? projects.find((x) => x.id === (spec.target as { project_id: string }).project_id)?.name : spec.target.kind === "temp" ? "scratch folder" : null;
  return (
    <span className="flex flex-wrap gap-1">
      {folder && <Token icon="folder">{folder}</Token>}
      <Token>{spec.harness && <HarnessMark harness={spec.harness} size={11} />}{profile?.name ?? spec.profile ?? "no profile"}</Token>
      {spec.model && spec.model !== profile?.model && <Token>{spec.model}</Token>}
      {spec.target.kind === "project" && !spec.worktree && <Token>checkout</Token>}
      {spec.worktree?.merge_when_done && <Token>merge when done</Token>}
      {spec.github_refs.length > 0 && <Token icon="pr">{spec.github_refs.length} GitHub item{spec.github_refs.length === 1 ? "" : "s"}</Token>}
      {(spec.agent_traces ?? []).length > 0 && <Token icon="agents">+ {spec.agent_traces.length} trace</Token>}
      {profile?.billing === "pay_as_you_go" && <Token tone="warn">pay as you go</Token>}
      {spec.budget_usd != null && <Token tone="warn">limit ${spec.budget_usd.toFixed(2)}</Token>}
    </span>
  );
}

/** Puts a proposal's spec into the composer draft (asks first when an unsent draft exists). */
function editInComposer(p: Proposal) {
  const spec = specOf(p);
  if (!spec) return;
  const c = useComposer.getState();
  useComposer.setState({ open: true, minimized: false });
  c.update((s) => ({ ...s, ...spec }));
  if (p.body.kind === "schedule") c.setConditions(p.body.conditions);
  if (p.body.kind === "cron") toast(`Pick Repeat on a schedule… in Launch ▾ (cron ${p.body.schedule})`);
}

function EditButton({ p }: { p: Proposal }) {
  const hasDraft = useComposer((s) => s.open && (!!s.spec.text.trim() || s.spec.github_refs.length > 0));
  if (!specOf(p)) return null;
  if (hasDraft) return <ConfirmButton tone="publish" consequence="Replace the current draft?" confirmLabel="Replace draft" onConfirm={() => editInComposer(p)}>Edit</ConfirmButton>;
  return <button className="btn btn-sm" title="Open it in the composer to change anything" onClick={() => editInComposer(p)}>Edit</button>;
}

const SessionRow = memo(function SessionRow({ s }: { s: Session }) {
  const show = useLayout((x) => x.show);
  const navigate = useNavigate();
  return (
    <button className="flex min-h-10 w-full items-center gap-2 rounded-md px-2 text-left hover:bg-sunk" onClick={() => { show(s.id); void navigate({ to: "/agents", search: { session: s.id } }); }}>
      <span className="min-w-0 flex-1"><span className="block truncate text-[13px] font-semibold">{s.title}</span>{s.last_message && <span className="block truncate text-[12px] text-ink-3">{s.last_message}</span>}</span>
      <StatusBadge s={s} />
    </button>
  );
});

const ProposalCard = memo(function ProposalCard({ p }: { p: Proposal }) {
  const qc = useQueryClient();
  const sessions = useSessions().data ?? [];
  const [busy, setBusy] = useState(false);
  const act = async (action: "accept" | "dismiss" | "recheck") => {
    setBusy(true);
    try {
      const next = await api.post<Proposal>(`/api/proposals/${p.id}/${action}`);
      qc.setQueryData<Proposal[]>(["proposals"], (old) => old?.map((x) => (x.id === next.id ? next : x)));
      if (next.state === "failed") toastError(new Error(next.error ?? "It could not start"));
    } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  const spec = specOf(p);
  const started = p.state === "started";
  const session = started && (p.body.kind === "launch" || p.body.kind === "message") ? sessions.find((s) => s.id === p.result_id) : undefined;
  const statusSessions = p.body.kind === "status" ? p.body.session_ids.map((id) => sessions.find((s) => s.id === id)).filter((s): s is Session => !!s) : [];
  return (
    <article className={`grid gap-2 rounded-xl border bg-paper p-3 ${p.body.kind === "cron" && !started ? "border-attn-line" : "border-line"}`}>
      <div className="flex items-start gap-2">
        <Icon name={p.body.kind === "message" ? "send" : p.body.kind === "schedule" ? "clock" : p.body.kind === "cron" ? "refresh" : p.body.kind === "status" ? "agents" : "play"} size={14} className="mt-0.5 flex-none text-ink-3" />
        <div className="min-w-0 flex-1">
          <b className="block text-[13px]">{p.title}</b>
          {p.note && <p className="m-0 text-[12px] text-ink-2">{p.note}</p>}
        </div>
      </div>
      {spec && <SpecTokens spec={spec} />}
      {p.body.kind === "cron" && <Token tone="warn" icon="refresh">Creates a repeating agent · {p.body.schedule}</Token>}
      {p.body.kind === "schedule" && <Token tone="warn" icon="clock">{p.body.conditions.length} start condition{p.body.conditions.length === 1 ? "" : "s"}</Token>}
      {spec && !started && <p className="m-0 line-clamp-3 text-[12px] whitespace-pre-wrap text-ink-2">{spec.text}</p>}
      {p.body.kind === "message" && !started && <p className="m-0 line-clamp-3 text-[12px] whitespace-pre-wrap text-ink-2">To “{sessions.find((s) => s.id === (p.body as { session_id: string }).session_id)?.title ?? "an agent"}”: {p.body.text}</p>}
      {statusSessions.length > 0 && <div className="grid">{statusSessions.map((s) => <SessionRow key={s.id} s={s} />)}</div>}
      {session && <SessionRow s={session} />}
      {started && !session && p.body.kind !== "status" && <span className="text-[12px] text-gh-open"><Icon name="check" size={11} /> {p.body.kind === "schedule" ? "Scheduled: see Agents → Scheduled" : p.body.kind === "cron" ? "Cron created: see Agents → Cron" : "Started"}</span>}
      {p.state === "failed" && p.error && <ErrorState error={p.error} />}
      {p.state === "uncertain" && <div className="grid gap-1.5 text-[12px] text-attn">Delivery not confirmed. Check Agents before starting a new attempt.<button className="btn btn-sm w-fit" disabled={busy} onClick={() => void act("recheck")}>I checked Agents — check again</button></div>}
      {(p.state === "proposed" || p.state === "failed") && p.body.kind !== "status" && (
        <div className="flex flex-wrap items-center gap-1.5">
          <button className="btn btn-sm btn-primary" disabled={busy} onClick={() => void act("accept")}><Icon name="play" size={12} />{busy ? "Starting…" : START[p.body.kind]}</button>
          <EditButton p={p} />
          <button className="btn btn-sm btn-ghost ml-auto" disabled={busy} onClick={() => void act("dismiss")}>Dismiss</button>
        </div>
      )}
      {p.body.kind === "status" && p.state === "proposed" && <button className="btn btn-sm btn-ghost w-fit" onClick={() => void act("dismiss")}>Dismiss</button>}
    </article>
  );
});

function useAssistant() {
  const config = useConfig().data?.config;
  const profiles = useProfiles();
  const current = profiles.find((p) => p.id === config?.assistant.profile) ?? profiles.find((p) => p.billing === "subscription") ?? profiles[0];
  const models = useHarnessDefaults(current?.models_url ? null : current?.harness ?? null).data?.models ?? [];
  const model = config?.assistant.model ?? null;
  return { profiles, current, models, model, modelName: model ? models.find((m) => m.id === model)?.display_name || model : null };
}

/** The assistant's profile and model choices (a daemon setting, all devices), kept apart from the composer's
 *  remembered defaults. Changing the model of a thread in Summary → Run also saves it here. */
function AssistantOptions({ close }: { close: () => void }) {
  const qc = useQueryClient();
  const { profiles, current, models, model } = useAssistant();
  const save = async (patch: Record<string, string>) => {
    close();
    try { qc.setQueryData(["config"], await api.patch<ConfigResponse>("/api/config", patch)); toast("The next thread uses it"); } catch (e) { toastError(e); }
  };
  return <>
    <div className="seclabel px-2 pt-1 pb-0.5">Assistant profile (new threads)</div>
    {profiles.map((p) => <MenuItem key={p.id} label={p.name} detail={p.billing === "pay_as_you_go" ? "Pay as you go" : "Subscription"} active={p.id === current?.id} onClick={() => void save({ assistant_profile: p.id, assistant_model: "", assistant_effort: "" })} />)}
    <div className="seclabel border-t border-line-2 px-2 pt-2 pb-0.5">Model (a fast model keeps replies quick)</div>
    <MenuItem label={`Default${current?.model ? ` (${current.model})` : ""}`} detail={current ? `Default of ${current.name}` : undefined} active={!model} onClick={() => void save({ assistant_model: "", assistant_effort: "" })} />
    {models.map((m) => <MenuItem key={m.id} label={m.display_name || m.id} detail={m.id} active={model === m.id} onClick={() => void save({ assistant_model: m.id, assistant_effort: "" })} />)}
    {models.length === 0 && <label className="grid gap-1 p-2 text-[12px]">
      Model ID
      <input className="field-input h-[28px] font-mono text-xs" defaultValue={model ?? ""} placeholder={current?.model ?? "profile default"} onKeyDown={(e) => { if (e.key === "Enter") void save({ assistant_model: e.currentTarget.value.trim() }); }} />
    </label>}
  </>;
}

function AssistantSettings() {
  const { current, modelName } = useAssistant();
  return (
    <Popover align="right" width={300} trigger={({ open, toggle }) => (
      <button className="btn btn-sm max-w-[280px]" aria-expanded={open} title="The profile and model of the assistant (new threads)" onClick={toggle}>{current && <HarnessMark harness={current.harness} size={13} />}<span className="min-w-0 truncate">{current?.name ?? "No profile"}{modelName ? ` · ${modelName}` : ""}</span><Icon name="chev" size={11} className="shrink-0" /></button>
    )}>
      {(close) => <AssistantOptions close={close} />}
    </Popover>
  );
}

/** Phone: one row with the Chat · Proposals tabs and a ⋯ menu (assistant model, New thread, composer). */
function PhoneAskBar({ pane, setPane, waiting, thread, newThread }: { pane: "chat" | "cards"; setPane: (p: "chat" | "cards") => void; waiting: number; thread: boolean; newThread: () => void }) {
  const { current, modelName } = useAssistant();
  return (
    <div className="flex shrink-0 items-center gap-1 border-b border-line px-3 pt-1">
      <Tabs className="min-w-0 flex-1" label="Ask views" value={pane} onChange={setPane} items={[{ id: "chat", label: "Chat" }, { id: "cards", label: "Proposals", count: waiting || undefined }]} />
      <PhoneActions label="Ask Launchpad actions" closeOnAction={false} details={<p className="m-0 text-[12px] text-ink-3">Assistant: {current?.name ?? "no profile"}{modelName ? ` · ${modelName}` : ""}. Proposals start only when you tap them; nothing reaches GitHub.</p>}>
        {(close: () => void) => <>
          {thread && <MenuItem icon={<Icon name="plus" size={16} />} label="New thread" detail="Archive this thread; it stays resumable in Agents" onClick={() => { close(); newThread(); }} />}
          <MenuItem icon={<Icon name="play" size={16} />} label="Open the composer" detail="Launch an agent yourself" onClick={() => { close(); useComposer.getState().start(); }} />
          <div className="border-t border-line-2 pt-1"><AssistantOptions close={close} /></div>
        </>}
      </PhoneActions>
    </div>
  );
}

/** First message of a thread (later messages use the session's own message box). */
const EXAMPLES = ["What needs me?", "Review the PRs waiting for me", "Summarize what my agents did today"];

function AskField({ onSend, busy, autoFocus, phone }: { onSend: (text: string) => void; busy: boolean; autoFocus?: boolean; phone?: boolean }) {
  const [text, setText] = useState("");
  return (<>
    {phone && <div className="flex flex-wrap gap-1.5">{EXAMPLES.map((x) => <button key={x} className="btn btn-sm" disabled={busy} onClick={() => setText(x)}>{x}</button>)}</div>}
    <div className="session-message-input flex items-end gap-2 rounded-lg border border-line px-2.5 py-[7px] focus-within:border-teal focus-within:shadow-[0_0_0_3px_var(--color-teal-soft)]">
      <textarea autoFocus={autoFocus} rows={2} value={text} disabled={busy} aria-label="Ask Launchpad" placeholder={phone ? "Ask or describe work…" : "Ask or describe work… e.g. “Fix the flaky snapshot test in hub”, “Review the PRs waiting for me”, “What needs me?”"} onChange={(e) => setText(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) { e.preventDefault(); if (text.trim()) { onSend(text.trim()); setText(""); } } }} className="max-h-40 min-h-10 min-w-0 flex-1 resize-none bg-transparent text-[13px] leading-5 outline-none" />
      <button className="iconbtn border-line! bg-paper" title="Send" aria-label="Send" disabled={busy || !text.trim()} onClick={() => { onSend(text.trim()); setText(""); }}><Icon name="send" size={14} /></button>
    </div>
  </>);
}

export function AskView() {
  const phone = usePhone();
  const qc = useQueryClient();
  const navigate = useNavigate();
  const workspace = useWorkspace((s) => s.current) ?? null;
  const scope = useScope();
  const sessions = useSessions().data ?? [];
  const proposals = useProposalList();
  const search = useSearch({ strict: false }) as { q?: string };
  const [busy, setBusy] = useState(false);
  const [filter, setFilter] = useState<"open" | "all">("open");
  const [pane, setPane] = useState<"chat" | "cards">("chat");
  const thread = sessions.filter((s) => s.launch_spec.target.kind === "assistant" && ((s.launch_spec.target as { workspace_id: string | null }).workspace_id ?? null) === workspace && s.status !== "closed").sort((a, b) => b.started_at - a.started_at)[0];
  const mine = (proposals.data ?? []).filter((p) => (p.workspace_id ?? null) === workspace);
  const waiting = mine.filter((p) => p.state === "proposed" || p.state === "failed" || p.state === "uncertain");
  const shown = filter === "open" ? waiting : mine.slice(0, 50);
  const ask = async (text: string) => {
    setBusy(true);
    try { await api.post<Session>("/api/assistant", { workspace_id: workspace, text }); await qc.invalidateQueries({ queryKey: ["sessions"] }); }
    catch (e) { toastError(e); } finally { setBusy(false); }
  };
  // The Agents dashboard field opens this page with its text already sent.
  const sent = useRef(false);
  useEffect(() => {
    if (!search.q || sent.current) return;
    sent.current = true;
    void ask(search.q).then(() => navigate({ to: "/ask", search: {} as never, replace: true }));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [search.q]);
  const startable = waiting.filter((p) => p.state === "proposed" && p.body.kind === "launch");
  const startAll = async () => {
    setBusy(true);
    try { for (const p of startable) await api.post(`/api/proposals/${p.id}/accept`); await qc.invalidateQueries({ queryKey: ["proposals"] }); }
    catch (e) { toastError(e); } finally { setBusy(false); }
  };
  const newThread = async () => {
    if (!thread) return;
    try { await api.post(`/api/sessions/${thread.id}/close`); toast("Thread archived; it stays resumable in Agents → Archived"); } catch (e) { toastError(e); }
  };
  const needs = sessions.filter((s) => scope.hasSession(s) && needsYou(s)).length;
  const running = sessions.filter((s) => scope.hasSession(s) && (isRunning(s) || isQuotaPaused(s))).length;
  const header = phone ? <PhoneAskBar pane={pane} setPane={setPane} waiting={waiting.length} thread={!!thread} newThread={() => void newThread()} /> : (
    <PageHeader
      icon={<Icon name="spark" size={18} className="text-teal-ink" />}
      title="Ask Launchpad"
      meta={<>{scope.workspace?.name ?? "Everything"} · proposals start only when you tap them · {needs} need you · {running} running</>}
      actions={<>
        <AssistantSettings />
        {thread && <button className="btn btn-sm" title="Archive this thread and start a new one" onClick={() => void newThread()}><Icon name="plus" size={12} />New thread</button>}
      </>}
    />
  );
  const cards = (
    <section className="flex min-h-0 flex-col gap-2 overflow-y-auto p-3">
      <div className="flex flex-wrap items-center gap-2">
        <FilterChips label="Proposals shown">
          <FilterChip active={filter === "open"} count={waiting.length} onClick={() => setFilter("open")}>Waiting</FilterChip>
          <FilterChip active={filter === "all"} onClick={() => setFilter("all")}>All</FilterChip>
        </FilterChips>
        {startable.length > 1 && <button className="btn btn-sm btn-primary ml-auto" disabled={busy} onClick={() => void startAll()}><Icon name="play" size={12} />Start all ({startable.length})</button>}
      </div>
      {proposals.isLoading ? <Loading what="proposals" /> : proposals.error ? <ErrorState error={proposals.error} onRetry={() => void proposals.refetch()} /> : shown.length === 0 ? <Empty icon="spark">{filter === "open" ? "No proposal waits for you." : "No proposal yet."}</Empty> : shown.map((p) => <ProposalCard key={p.id} p={p} />)}
    </section>
  );
  const chat = thread
    ? <div className="relative min-h-0 flex-1"><SessionPane chat key={thread.id} sessionId={thread.id} onClear={() => void navigate({ to: "/agents" })} /></div>
    : phone
      // Phone: the intro at the top and the field above the bottom tabs, like the session message box.
      ? <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-3">
          <p className="m-0 text-[13px] text-ink-2">Describe work in plain words. The assistant reads your folders, agents, inbox and quotas, then proposes launches, messages, schedules and crons. Nothing starts until you tap it; nothing reaches GitHub.</p>
          {busy && <Loading what="the assistant" />}
          <div className="mt-auto grid gap-2"><AskField phone onSend={(t) => void ask(t)} busy={busy} /></div>
        </div>
      : <div className="grid min-h-0 flex-1 place-items-center p-6">
        <div className="grid w-full max-w-[560px] gap-3">
          <p className="m-0 text-[13px] text-ink-2">Describe work in plain words. The assistant reads your folders, agents, inbox and quotas, then proposes launches, messages, schedules and crons as cards. Nothing starts until you tap it; nothing reaches GitHub.</p>
          <AskField onSend={(t) => void ask(t)} busy={busy} autoFocus />
          {busy && <Loading what="the assistant" />}
          <p className="m-0 text-[12px] text-ink-3">The full composer stays one click away: <button className="text-teal-ink hover:underline" onClick={() => useComposer.getState().start()}>open the composer</button> or use <Link to="/agents">Agents</Link>.</p>
        </div>
      </div>;
  if (phone)
    return (
      <div className="flex h-full min-h-0 flex-col">
        {header}
        {pane === "chat" ? <>{waiting.length > 0 && <button className="flex min-h-11 shrink-0 items-center gap-2 border-b border-line bg-teal-soft px-3 text-left text-[13px] font-semibold text-teal-ink" onClick={() => setPane("cards")}><Icon name="spark" size={14} />{waiting.length} proposal{waiting.length === 1 ? "" : "s"} waiting for you<Icon name="chev" size={14} className="ml-auto -rotate-90" /></button>}{chat}</> : cards}
      </div>
    );
  return (
    <div className="flex h-full min-h-0 flex-col">
      {header}
      <div className="grid min-h-0 flex-1 grid-cols-[minmax(0,1fr)_360px]">
        <div className="flex min-h-0 flex-col border-r border-line">{chat}</div>
        {cards}
      </div>
    </div>
  );
}
