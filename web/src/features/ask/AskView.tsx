// Ask Launchpad: a chat with the workspace's assistant session that turns requests into proposal cards.
// Cards are inert until the owner clicks Start (or Send, Schedule, Create cron); started cards become live
// status rows bound to their session through the event bus. The thread is a normal session (SessionPane).

import { useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate, useSearch } from "@tanstack/react-router";
import { useEffect, useRef, useState, type ReactNode } from "react";
import type { Session } from "@/gen/Session";
import type { FileAttachment } from "@/gen/FileAttachment";
import { HarnessMark, useHarnessInfo } from "@/components/HarnessMark";
import { Icon } from "@/components/Icon";
import { MenuItem } from "@/components/Popover";
import { toastError } from "@/components/Toast";
import { Badge, Empty, ErrorState, FilterChip, FilterChips, Loading, PageHeader, Tabs } from "@/components/ui";
import { api } from "@/lib/api";
import { pastedImages } from "@/lib/attachments";
import { usePhone } from "@/lib/phone";
import { useProfiles } from "@/lib/profiles";
import { useConfig, useCrons, useHarnessDefaults, useSessions } from "@/lib/queries";
import { needsYou, isQuotaPaused, isRunning } from "@/lib/sessionStatus";
import { useScope } from "@/lib/workspace";
import { useComposer } from "@/stores/composer";
import { useWorkspace } from "@/stores/workspace";
import { isWaiting, visibleProposals } from "./proposalState";
import { ProposalCard, useProposalList } from "./ProposalCard";
import { PresetButton, PresetChips, usePresets } from "@/features/agents/MessagePresets";
import { withPresets } from "@/lib/presets";
import { SessionPane } from "@/features/agents/SessionPane";
import { PhoneActions } from "@/mobile/PhoneHeader";
import { ATTACH_LIMITS, AttachmentList, useMessageAttachments } from "@/features/agents/MessageAttachments";
import { AskInstructionsPanel, AskRoutinesPanel, AskSettingsPanel, AskThreadsPanel, askRoutines } from "./AskPanels";
import { useAskBot } from "@/lib/bots";

function useAssistant() {
  const config = useConfig().data?.config;
  const profiles = useProfiles();
  const current = profiles.find((p) => p.id === config?.assistant.profile) ?? profiles.find((p) => p.billing === "subscription") ?? profiles[0];
  const models = useHarnessDefaults(current?.models_url ? null : current?.harness ?? null).data?.models ?? [];
  const model = config?.assistant.model ?? null;
  return { profiles, current, models, model, modelName: model ? models.find((m) => m.id === model)?.display_name || model : null };
}

type Pane = "chat" | "cards" | "routines" | "instructions" | "threads" | "settings";
const PANES: Pane[] = ["cards", "routines", "instructions", "threads", "settings"];

/** Phone: the tabs of the page (Chat first) and a ⋯ menu with New thread and the composer. */
function PhoneAskBar({ tabs, thread, newThread }: { tabs: ReactNode; thread: boolean; newThread: () => void }) {
  const { current, modelName } = useAssistant();
  const auto = useConfig().data?.config.assistant.auto_mode ?? false;
  return (
    <div className="flex shrink-0 items-center gap-1 border-b border-line px-3 pt-1">
      <div className="min-w-0 flex-1">{tabs}</div>
      <PhoneActions label="Ask Launchpad actions" closeOnAction={false} details={<p className="m-0 text-[12px] text-ink-3">Assistant: {current?.name ?? "no profile"}{modelName ? ` · ${modelName}` : ""}. {auto ? "Auto mode: new proposals start immediately" : "Manual mode: proposals wait for your tap"}; nothing reaches GitHub. Change it in Settings.</p>}>
        {(close: () => void) => <>
          {thread && <MenuItem icon={<Icon name="plus" size={16} />} label="New thread" detail="Archive this thread; it stays resumable in Agents" onClick={() => { close(); newThread(); }} />}
          <MenuItem icon={<Icon name="play" size={16} />} label="Open the composer" detail="Launch an agent yourself" onClick={() => { close(); useComposer.getState().start(); }} />
        </>}
      </PhoneActions>
    </div>
  );
}

/** First message of a thread (later messages use the session's own message box). */
const EXAMPLES = ["What needs me?", "Review the PRs waiting for me", "Summarize what my agents did today"];

function AskField({ onSend, busy, autoFocus, phone }: { onSend: (text: string, attachments: FileAttachment[]) => Promise<boolean>; busy: boolean; autoFocus?: boolean; phone?: boolean }) {
  const [text, setText] = useState("");
  const [presets, setPresets] = useState<string[]>([]);
  const allPresets = usePresets("ask_launchpad");
  const message = withPresets(text, presets, allPresets);
  // The profile the next thread starts with decides whether images can be attached.
  const images = useHarnessInfo(useAssistant().current?.harness ?? null)?.capabilities.images ?? true;
  const attachments = useMessageAttachments(images);
  const { files, reading } = attachments;
  const fileRef = useRef<HTMLInputElement>(null);
  const [dragging, setDragging] = useState(false);
  const blocked = busy || reading;
  const send = async () => {
    if (blocked || (!message && !files.length)) return;
    if (await onSend(message, files)) { setText(""); setPresets([]); attachments.clear(); }
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
      <PresetChips surface="ask_launchpad" selected={presets} onChange={setPresets} disabled={blocked} className="mb-1.5" />
      <div className="session-message-input flex items-end gap-2 rounded-lg border border-line px-2.5 py-[7px] focus-within:border-teal focus-within:shadow-[0_0_0_3px_var(--color-teal-soft)]">
        <textarea autoFocus={autoFocus} rows={2} value={text} disabled={blocked} aria-label="Ask Launchpad" placeholder={phone ? "Ask or describe work…" : "Ask or describe work… e.g. “Fix the flaky snapshot test in hub”, “Review the PRs waiting for me”, “What needs me?”"}
          onChange={(e) => setText(e.target.value)}
          onPaste={(e) => { if (!images || blocked || attachments.readingRef.current) return; const pasted = pastedImages(e); if (pasted) void attachments.add(pasted); }}
          onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) { e.preventDefault(); void send(); } }}
          className="max-h-40 min-h-10 min-w-0 flex-1 resize-none bg-transparent text-[13px] leading-5 outline-none" />
        <PresetButton surface="ask_launchpad" selected={presets} onChange={setPresets} disabled={blocked} />
        <button className="iconbtn" title={`Attach files or images (you can also drop files${images ? " or paste a screenshot" : ""}). ${ATTACH_LIMITS}`} aria-label="Attach files or images" disabled={blocked} onClick={() => fileRef.current?.click()}><Icon name="clip" size={14} /></button>
        <button className="iconbtn border-line! bg-paper" title="Send" aria-label="Send" disabled={blocked || (!message && !files.length)} onClick={() => void send()}><Icon name="send" size={14} /></button>
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
  const search = useSearch({ strict: false }) as { q?: string; proposal?: string; panel?: string };
  const [busy, setBusy] = useState(false);
  const [filter, setFilter] = useState<"open" | "all">("open");
  const [pane, setPane] = useState<Pane>(search.proposal ? "cards" : PANES.find((p) => p === search.panel) ?? (phone ? "chat" : "cards"));
  useEffect(() => { const p = PANES.find((x) => x === search.panel); if (p) setPane(p); }, [search.panel]);
  const routines = askRoutines(useCrons().data ?? [], workspace);
  const askBot = useAskBot().data;
  const { current, modelName } = useAssistant();
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
    try { for (const p of startable) await api.post(`/api/proposals/${p.id}/accept`); await Promise.all([qc.invalidateQueries({ queryKey: ["proposals"] }), qc.invalidateQueries({ queryKey: ["sessions"] })]); if (phone) setPane("chat"); }
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
  const tabs = <Tabs label="Ask Launchpad panels" value={pane} onChange={setPane} tour="ask.panels" items={[...(phone ? [{ id: "chat" as const, label: "Chat" }] : []), { id: "cards", label: "Proposals", count: waiting.length || undefined }, { id: "routines", label: "Routines", count: routines.length || undefined }, { id: "instructions", label: "Instructions" }, { id: "threads", label: "Threads" }, { id: "settings", label: "Settings" }]} />;
  const header = phone ? <PhoneAskBar tabs={tabs} thread={!!thread} newThread={() => void newThread()} /> : (
    <PageHeader
      icon={<Icon name="spark" size={18} className="text-teal-ink" />}
      title="Ask Launchpad"
      meta={<>
        <Badge title="The built-in bot: one thread per workspace, proposals only">Built-in bot</Badge>
        {askBot?.customized && <Badge tone="teal" title="BOT.md or the settings differ from the defaults (Settings → Reset)">Customized</Badge>}
        <span>{scope.workspace?.name ?? "Everything"}</span>
        <span className="inline-flex items-center gap-1">{current && <HarnessMark harness={current.harness} size={12} />}{current?.name ?? "no profile"}{modelName ? ` · ${modelName}` : ""}</span>
        <span>{auto ? "Auto mode" : "Manual mode"}</span>
        <span>{needs} need you · {running} running</span>
      </>}
      actions={thread && <button className="btn btn-sm" title="Archive this thread and start a new one" onClick={() => void newThread()}><Icon name="plus" size={12} />New thread</button>}
    />
  );
  const cards = (
    <section data-tour="ask.proposals" className="flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto p-3">
      <div className="flex flex-wrap items-center gap-2">
        <FilterChips label="Proposals shown">
          <FilterChip active={filter === "open"} count={waiting.length} onClick={() => setFilter("open")}>Waiting</FilterChip>
          <FilterChip active={filter === "all"} onClick={() => setFilter("all")}>All</FilterChip>
        </FilterChips>
        {startable.length > 1 && <button className="btn btn-sm btn-primary ml-auto" disabled={busy} onClick={() => void startAll()}><Icon name="play" size={12} />Start all ({startable.length})</button>}
      </div>
      {proposals.isLoading ? <Loading what="proposals" /> : proposals.error ? <ErrorState error={proposals.error} onRetry={() => void proposals.refetch()} /> : shown.length === 0 ? <Empty icon="spark">{filter === "open" ? "No proposal waits for you." : "No proposal yet."}</Empty> : shown.map((p) => <ProposalCard key={p.id} p={p} anchor onAccepted={() => { if (phone) setPane("chat"); }} />)}
    </section>
  );
  const chat = thread
    ? <div data-tour="ask.chat" className="relative min-h-0 flex-1"><SessionPane chat key={thread.id} sessionId={thread.id} onClear={() => void navigate({ to: "/agents" })} /></div>
    : phone
      // Phone: the intro at the top and the field above the bottom tabs, like the session message box.
      ? <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-3">
          <p className="m-0 text-[13px] text-ink-2">Describe work in plain words. The assistant reads your folders, agents, inbox and quotas, then proposes launches, messages, schedules, crons and bots. {auto ? "Auto mode starts new proposals immediately." : "Proposals wait for your tap."} Nothing reaches GitHub.</p>
          {resumePrevious}
          {busy && <Loading what="the assistant" />}
          <div className="mt-auto grid gap-2"><AskField phone onSend={ask} busy={busy} /></div>
        </div>
      : <div className="grid min-h-0 flex-1 place-items-center p-6">
        <div className="grid w-full max-w-[560px] gap-3">
          <p className="m-0 text-[13px] text-ink-2">Describe work in plain words. The assistant reads your folders, agents, inbox and quotas, then proposes launches, messages, schedules, crons and bots as cards. {auto ? "Auto mode starts new proposals immediately." : "Proposals wait for your tap."} Nothing reaches GitHub.</p>
          <AskField onSend={ask} busy={busy} autoFocus />
          {resumePrevious}
          {busy && <Loading what="the assistant" />}
          <p className="m-0 text-[12px] text-ink-3">The full composer stays one click away: <button className="text-teal-ink hover:underline" onClick={() => useComposer.getState().start()}>open the composer</button> or use <Link to="/agents">Agents</Link>.</p>
        </div>
      </div>;
  const side = pane === "cards" || pane === "chat" ? cards : <div className="min-h-0 flex-1 overflow-y-auto">
    {pane === "routines" && <AskRoutinesPanel routines={routines} workspace={workspace} />}
    {pane === "instructions" && <AskInstructionsPanel />}
    {pane === "threads" && <AskThreadsPanel threads={threads} />}
    {pane === "settings" && <AskSettingsPanel />}
  </div>;
  if (phone)
    return (
      <div className="flex h-full min-h-0 flex-col">
        {header}
        {pane === "chat" ? <>{waiting.length > 0 && <button className="flex min-h-11 shrink-0 items-center gap-2 border-b border-line bg-teal-soft px-3 text-left text-[13px] font-semibold text-teal-ink" onClick={() => setPane("cards")}><Icon name="spark" size={14} />{waiting.length} proposal{waiting.length === 1 ? "" : "s"} waiting for you<Icon name="chev" size={14} className="ml-auto -rotate-90" /></button>}{chat}</> : side}
      </div>
    );
  return (
    <div className="flex h-full min-h-0 flex-col">
      {header}
      <div className="grid min-h-0 flex-1 grid-cols-[minmax(0,1fr)_440px]">
        <div className="flex min-h-0 flex-col border-r border-line">{chat}</div>
        <aside className="flex min-h-0 flex-col">
          <div className="border-b border-line px-3 pt-1">{tabs}</div>
          {side}
        </aside>
      </div>
    </div>
  );
}
