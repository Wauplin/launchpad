// Ask Launchpad: a chat with the workspace's assistant session that turns requests into proposal cards.
// Cards are inert until the owner clicks Start (or Send, Schedule, Create cron); started cards become live
// status rows bound to their session through the event bus. The thread is a normal session (SessionPane).

import { useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate, useSearch } from "@tanstack/react-router";
import { useEffect, useRef, useState } from "react";
import type { Session } from "@/gen/Session";
import type { FileAttachment } from "@/gen/FileAttachment";
import { HarnessMark, useHarnessInfo } from "@/components/HarnessMark";
import { Icon } from "@/components/Icon";
import { MenuItem, Popover } from "@/components/Popover";
import { toast, toastError } from "@/components/Toast";
import { Empty, ErrorState, FilterChip, FilterChips, Loading, PageHeader, Tabs } from "@/components/ui";
import { api } from "@/lib/api";
import { pastedImages } from "@/lib/attachments";
import { usePhone } from "@/lib/phone";
import { useProfiles } from "@/lib/profiles";
import { useConfig, useHarnessDefaults, useSessions, type ConfigResponse } from "@/lib/queries";
import { needsYou, isQuotaPaused, isRunning } from "@/lib/sessionStatus";
import { useScope } from "@/lib/workspace";
import { useComposer } from "@/stores/composer";
import { useWorkspace } from "@/stores/workspace";
import { isWaiting, visibleProposals } from "./proposalState";
import { ProposalCard, useProposalList } from "./ProposalCard";
import { SessionPane } from "@/features/agents/SessionPane";
import { PhoneActions } from "@/mobile/PhoneHeader";
import { ATTACH_LIMITS, AttachmentList, useMessageAttachments } from "@/features/agents/MessageAttachments";

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
  const auto = useConfig().data?.config.assistant.auto_mode ?? false;
  const save = async (patch: Record<string, string | boolean>) => {
    close();
    try { qc.setQueryData(["config"], await api.patch<ConfigResponse>("/api/config", patch)); toast("Ask settings saved"); } catch (e) { toastError(e); }
  };
  return <>
    <div className="seclabel px-2 pt-1 pb-0.5">Proposal mode</div>
    <MenuItem label="Manual" detail="Review each proposal before it starts" active={!auto} onClick={() => void save({ assistant_auto_mode: false })} />
    <MenuItem label="Auto" detail="New proposals start immediately, including messages and crons" active={auto} onClick={() => void save({ assistant_auto_mode: true })} />
    <div className="seclabel border-t border-line-2 px-2 pt-2 pb-0.5">Assistant profile (new threads)</div>
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
  const auto = useConfig().data?.config.assistant.auto_mode ?? false;
  return (
    <Popover align="right" width={300} trigger={({ open, toggle }) => (
      <button className="btn btn-sm max-w-[280px]" aria-expanded={open} title="The profile and model of the assistant (new threads)" onClick={toggle}>{current && <HarnessMark harness={current.harness} size={13} />}<span className="min-w-0 truncate">{current?.name ?? "No profile"}{modelName ? ` · ${modelName}` : ""} · {auto ? "Auto" : "Manual"}</span><Icon name="chev" size={11} className="shrink-0" /></button>
    )}>
      {(close) => <AssistantOptions close={close} />}
    </Popover>
  );
}

/** Phone: one row with the Chat · Proposals tabs and a ⋯ menu (assistant model, New thread, composer). */
function PhoneAskBar({ pane, setPane, waiting, thread, newThread }: { pane: "chat" | "cards"; setPane: (p: "chat" | "cards") => void; waiting: number; thread: boolean; newThread: () => void }) {
  const { current, modelName } = useAssistant();
  const auto = useConfig().data?.config.assistant.auto_mode ?? false;
  return (
    <div className="flex shrink-0 items-center gap-1 border-b border-line px-3 pt-1">
      <Tabs className="min-w-0 flex-1" label="Ask views" value={pane} onChange={setPane} items={[{ id: "chat", label: "Chat" }, { id: "cards", label: "Proposals", count: waiting || undefined }]} />
      <PhoneActions label="Ask Launchpad actions" closeOnAction={false} details={<p className="m-0 text-[12px] text-ink-3">Assistant: {current?.name ?? "no profile"}{modelName ? ` · ${modelName}` : ""}. {auto ? "Auto mode: new proposals start immediately" : "Manual mode: proposals wait for your tap"}; nothing reaches GitHub.</p>}>
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

function AskField({ onSend, busy, autoFocus, phone }: { onSend: (text: string, attachments: FileAttachment[]) => Promise<boolean>; busy: boolean; autoFocus?: boolean; phone?: boolean }) {
  const [text, setText] = useState("");
  // The profile the next thread starts with decides whether images can be attached.
  const images = useHarnessInfo(useAssistant().current?.harness ?? null)?.capabilities.images ?? true;
  const attachments = useMessageAttachments(images);
  const { files, reading } = attachments;
  const fileRef = useRef<HTMLInputElement>(null);
  const [dragging, setDragging] = useState(false);
  const blocked = busy || reading;
  const send = async () => {
    if (blocked || (!text.trim() && !files.length)) return;
    if (await onSend(text.trim(), files)) { setText(""); attachments.clear(); }
  };
  return (<>
    {phone && <div className="flex flex-wrap gap-1.5">{EXAMPLES.map((x) => <button key={x} className="btn btn-sm" disabled={busy} onClick={() => setText(x)}>{x}</button>)}</div>}
    <div className="relative"
      onDragOver={(e) => { if (e.dataTransfer.types.includes("Files")) { e.preventDefault(); setDragging(true); } }}
      onDragLeave={(e) => { if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setDragging(false); }}
      onDrop={(e) => { if (e.dataTransfer.types.includes("Files")) { e.preventDefault(); setDragging(false); if (!blocked) void attachments.add(Array.from(e.dataTransfer.files)); } }}
    >
      {dragging && <div className="pointer-events-none absolute inset-0 z-10 grid place-items-center rounded-lg border-2 border-teal bg-teal-soft/80 text-[13px] font-semibold text-teal-ink">Drop files{images ? " or images" : ""} to attach</div>}
      <input ref={fileRef} type="file" multiple hidden onChange={(e) => { void attachments.add(Array.from(e.target.files ?? [])); e.target.value = ""; }} />
      <AttachmentList files={files} disabled={blocked} onRemove={attachments.remove} />
      {reading && <div className="mb-1 text-[12px] text-ink-3">Reading attachments…</div>}
      <div className="session-message-input flex items-end gap-2 rounded-lg border border-line px-2.5 py-[7px] focus-within:border-teal focus-within:shadow-[0_0_0_3px_var(--color-teal-soft)]">
        <textarea autoFocus={autoFocus} rows={2} value={text} disabled={blocked} aria-label="Ask Launchpad" placeholder={phone ? "Ask or describe work…" : "Ask or describe work… e.g. “Fix the flaky snapshot test in hub”, “Review the PRs waiting for me”, “What needs me?”"}
          onChange={(e) => setText(e.target.value)}
          onPaste={(e) => { if (!images || blocked || attachments.readingRef.current) return; const pasted = pastedImages(e); if (pasted) void attachments.add(pasted); }}
          onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) { e.preventDefault(); void send(); } }}
          className="max-h-40 min-h-10 min-w-0 flex-1 resize-none bg-transparent text-[13px] leading-5 outline-none" />
        <button className="iconbtn" title={`Attach files or images (you can also drop files${images ? " or paste a screenshot" : ""}). ${ATTACH_LIMITS}`} aria-label="Attach files or images" disabled={blocked} onClick={() => fileRef.current?.click()}><Icon name="clip" size={14} /></button>
        <button className="iconbtn border-line! bg-paper" title="Send" aria-label="Send" disabled={blocked || (!text.trim() && !files.length)} onClick={() => void send()}><Icon name="send" size={14} /></button>
      </div>
    </div>
  </>);
}

export function AskView() {
  const phone = usePhone();
  const auto = useConfig().data?.config.assistant.auto_mode ?? false;
  const qc = useQueryClient();
  const navigate = useNavigate();
  const workspace = useWorkspace((s) => s.current) ?? null;
  const scope = useScope();
  const sessions = useSessions().data ?? [];
  const proposals = useProposalList();
  const search = useSearch({ strict: false }) as { q?: string; proposal?: string };
  const [busy, setBusy] = useState(false);
  const [filter, setFilter] = useState<"open" | "all">("open");
  const [pane, setPane] = useState<"chat" | "cards">(search.proposal ? "cards" : "chat");
  const revealed = search.proposal ?? null;
  const focusedProposal = useRef<string | null>(null);
  useEffect(() => {
    if (revealed) { setFilter("all"); setPane("cards"); }
  }, [revealed]);
  useEffect(() => {
    if (!revealed || filter !== "all" || focusedProposal.current === revealed) return;
    const card = document.getElementById(`proposal-${revealed}`);
    if (card) { card.focus(); focusedProposal.current = revealed; }
  }, [revealed, filter, proposals.data, pane]);
  const threads = (useSessions(true).data ?? []).filter((s) => s.launch_spec.target.kind === "assistant" && ((s.launch_spec.target as { workspace_id: string | null }).workspace_id ?? null) === workspace).sort((a, b) => b.started_at - a.started_at);
  const thread = threads.find((s) => s.status !== "closed");
  const previous = threads.find((s) => s.status === "closed");
  const mine = (proposals.data ?? []).filter((p) => (p.workspace_id ?? null) === workspace);
  const waiting = mine.filter(isWaiting);
  const shown = visibleProposals(mine, filter === "open", revealed);
  const ask = async (text: string, attachments: FileAttachment[] = []) => {
    setBusy(true);
    try { await api.post<Session>("/api/assistant", { workspace_id: workspace, text, attachments }); await qc.invalidateQueries({ queryKey: ["sessions"] }); return true; }
    catch (e) { toastError(e); return false; } finally { setBusy(false); }
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
    try { for (const p of startable) await api.post(`/api/proposals/${p.id}/accept`); await Promise.all([qc.invalidateQueries({ queryKey: ["proposals"] }), qc.invalidateQueries({ queryKey: ["sessions"] })]); setPane("chat"); }
    catch (e) { toastError(e); } finally { setBusy(false); }
  };
  const newThread = async () => {
    if (!thread) return;
    try { await api.post(`/api/sessions/${thread.id}/close`); await qc.invalidateQueries({ queryKey: ["sessions"] }); } catch (e) { toastError(e); }
  };
  const resume = async () => {
    if (!previous || busy) return;
    setBusy(true);
    try { await api.post(`/api/sessions/${previous.id}/resume`); await qc.invalidateQueries({ queryKey: ["sessions"] }); }
    catch (e) { toastError(e); } finally { setBusy(false); }
  };
  const resumePrevious = previous && !thread && <button className="btn btn-sm w-fit" disabled={busy} title={previous.title} onClick={() => void resume()}><Icon name="refresh" size={12} />Resume previous thread</button>;
  const needs = sessions.filter((s) => scope.hasSession(s) && needsYou(s)).length;
  const running = sessions.filter((s) => scope.hasSession(s) && (isRunning(s) || isQuotaPaused(s))).length;
  const header = phone ? <PhoneAskBar pane={pane} setPane={setPane} waiting={waiting.length} thread={!!thread} newThread={() => void newThread()} /> : (
    <PageHeader
      icon={<Icon name="spark" size={18} className="text-teal-ink" />}
      title="Ask Launchpad"
      meta={<>{scope.workspace?.name ?? "Everything"} · {auto ? "auto mode: new proposals start immediately" : "manual mode: proposals wait for your tap"} · {needs} need you · {running} running</>}
      actions={<>
        <AssistantSettings />
        {thread && <button className="btn btn-sm" title="Archive this thread and start a new one" onClick={() => void newThread()}><Icon name="plus" size={12} />New thread</button>}
      </>}
    />
  );
  const cards = (
    <section data-tour="ask.proposals" className="flex min-h-0 flex-col gap-2 overflow-y-auto p-3">
      <div className="flex flex-wrap items-center gap-2">
        <FilterChips label="Proposals shown">
          <FilterChip active={filter === "open"} count={waiting.length} onClick={() => setFilter("open")}>Waiting</FilterChip>
          <FilterChip active={filter === "all"} onClick={() => setFilter("all")}>All</FilterChip>
        </FilterChips>
        {startable.length > 1 && <button className="btn btn-sm btn-primary ml-auto" disabled={busy} onClick={() => void startAll()}><Icon name="play" size={12} />Start all ({startable.length})</button>}
      </div>
      {proposals.isLoading ? <Loading what="proposals" /> : proposals.error ? <ErrorState error={proposals.error} onRetry={() => void proposals.refetch()} /> : shown.length === 0 ? <Empty icon="spark">{filter === "open" ? "No proposal waits for you." : "No proposal yet."}</Empty> : shown.map((p) => <ProposalCard key={p.id} p={p} anchor onAccepted={() => setPane("chat")} />)}
    </section>
  );
  const chat = thread
    ? <div data-tour="ask.chat" className="relative min-h-0 flex-1"><SessionPane chat key={thread.id} sessionId={thread.id} onClear={() => void navigate({ to: "/agents" })} /></div>
    : phone
      // Phone: the intro at the top and the field above the bottom tabs, like the session message box.
      ? <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-3">
          <p className="m-0 text-[13px] text-ink-2">Describe work in plain words. The assistant reads your folders, agents, inbox and quotas, then proposes launches, messages, schedules and crons. {auto ? "Auto mode starts new proposals immediately." : "Proposals wait for your tap."} Nothing reaches GitHub.</p>
          {resumePrevious}
          {busy && <Loading what="the assistant" />}
          <div className="mt-auto grid gap-2"><AskField phone onSend={ask} busy={busy} /></div>
        </div>
      : <div className="grid min-h-0 flex-1 place-items-center p-6">
        <div className="grid w-full max-w-[560px] gap-3">
          <p className="m-0 text-[13px] text-ink-2">Describe work in plain words. The assistant reads your folders, agents, inbox and quotas, then proposes launches, messages, schedules and crons as cards. {auto ? "Auto mode starts new proposals immediately." : "Proposals wait for your tap."} Nothing reaches GitHub.</p>
          <AskField onSend={ask} busy={busy} autoFocus />
          {resumePrevious}
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
