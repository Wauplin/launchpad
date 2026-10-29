// One session in a pane: header with state and actions, the conversation, and the message box.
// The conversation is virtualized (spike 4 rules: memoized rows, no flushSync).

import { ProposalCard, useProposalList } from "@/features/ask/ProposalCard";
import { FollowUpAgent } from "./FollowUpAgent";
import { HostLabel } from "@/components/HostLabel";
import { hostOf, linkLabel, useHost } from "@/lib/hosts";
import { useProfile } from "@/lib/profiles";
import { SessionSummary, StateBand, QuotaBand, FollowUpBands, CostLimit, PaidUsageToggle, RunSettingsChip } from "./SessionSummary";
import { Tabs } from "@/components/ui";
import { SubagentsPanel, subagents } from "./SubagentsPanel";
import { GitPanel } from "./GitPanel";
import { ProcessesPanel } from "./ProcessesPanel";
import type { SessionProcess } from "@/gen/SessionProcess";
import { useComposer } from "@/stores/composer";
import { useLayout } from "@/stores/layout";
import { useQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { Session } from "@/gen/Session";
import { HarnessMark, useHarnessInfo } from "@/components/HarnessMark";
import { CodeLinksContext, ShellCodeContext, type CodeLinks } from "@/components/Markdown";
import { citationPath } from "@/lib/fileCitations";
import { daemonAsset } from "@/lib/connection";
import { downloadUrl, pathResolver, useCodeFiles } from "@/features/code/lib";
import { useMessageDrafts } from "@/stores/messages";
import { isDone, isRunning, sessionPlace } from "@/lib/sessionStatus";
import { Icon } from "@/components/Icon";
import { toastError } from "@/components/Toast";
import { abandonOperation } from "@/lib/operations";
import { api, ApiError } from "@/lib/api";
import { pastedImages } from "@/lib/attachments";
import { ATTACH_LIMITS, AttachmentList, useMessageAttachments } from "./MessageAttachments";
import { projectName, useProjects, useSession } from "@/lib/queries";
import { emptyLog, useSessionLogs } from "@/lib/sessionStore";
import { buildRows } from "./rows";
import { EventRow } from "./EventRow";
import { foldTrace } from "./traceRows";
import { MenuItem, Popover } from "@/components/Popover";
import type { SessionGitHistory } from "@/gen/SessionGitHistory";
import type { SessionGitStatus } from "@/gen/SessionGitStatus";
import { StatusBadge, usageLine } from "./SessionBits";
import { useOpenTerminal, useTerminals } from "@/features/terminals/lib";
import { TerminalMenu } from "@/features/terminals/TerminalMenu";
import { Sheet } from "@/mobile/Sheet";
import { WorktreeCard, useSessionWorktrees, WorktreeChip } from "./Worktrees";
import { usePhone } from "@/lib/phone";
import { PresetButton, PresetChips, usePresets } from "./MessagePresets";
import { withPresets } from "@/lib/presets";

function act(id: string, action: string) {
  return api.post(`/api/sessions/${id}/${action}`).catch(toastError);
}

type PendingAsk = { request_id: string; count: number; permission: boolean } | null;

/** The session message box; Now cards on the phone reuse it for Reply. */
export function MessageBox({ acceptsImages, sessionId, placeholder, disabled, ask, dropImages, inputRef: ref, onSent }: { acceptsImages: boolean; sessionId: string; placeholder: string; disabled: boolean; ask: PendingAsk; dropImages: React.RefObject<((files: File[]) => void) | null>; inputRef: React.RefObject<HTMLTextAreaElement | null>; onSent: () => void }) {
  const text = useMessageDrafts((s) => s.drafts[sessionId] ?? "");
  const setDraft = useMessageDrafts((s) => s.set);
  const setText = (t: string) => setDraft(sessionId, t);
  const allPresets = usePresets();
  const selectedPresets = useMessageDrafts((s) => s.presets[sessionId]);
  const setStoredPresets = useMessageDrafts((s) => s.setPresets);
  // Presets go with messages, not with answers to an open question.
  const presets = ask ? [] : (selectedPresets ?? []).filter((id) => allPresets.some((p) => p.id === id));
  const setPresets = (ids: string[]) => setStoredPresets(sessionId, ids);
  const message = ask ? text.trim() : withPresets(text, presets, allPresets);
  const [busy, setBusy] = useState(false);
  const [delivery, setDelivery] = useState<"sending" | "delivered" | "uncertain" | "failed" | null>(null);
  const attachments = useMessageAttachments(acceptsImages);
  const { files, reading, readingRef } = attachments;
  const fileRef = useRef<HTMLInputElement>(null);
  const addFiles = (input: File[] | Promise<File[]>) => {
    if (disabled || busy) return;
    void attachments.add(input);
  };
  useLayoutEffect(() => { dropImages.current = (dropped) => addFiles(dropped); });
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 160)}px`;
  }, [text, ref]);
  const send = async () => {
    const t = message;
    if ((!t && !files.length) || busy || disabled || reading || ask?.permission) return;
    setBusy(true);
    setDelivery("sending");
    try {
      if (ask) {
        // A typed answer to the open question.
        await api.post(`/api/sessions/${sessionId}/answer`, { request_id: ask.request_id, answers: Array(ask.count).fill(t || "See the attachments in my next message.") });
        if (files.length) await api.post(`/api/sessions/${sessionId}/input`, { text: t, files });
      } else {
        await api.post(`/api/sessions/${sessionId}/input`, { text: t, files });
      }
      setDelivery("delivered");
      onSent();
      setText("");
      if (!ask) setPresets([]);
      attachments.clear();
    } catch (e) {
      setDelivery(e instanceof ApiError && e.operationCompleted ? "failed" : "uncertain");
      toastError(e);
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="border-t border-line-2 px-2.5 pt-2 pb-2.5" data-tour="session.reply">
      <input ref={fileRef} type="file" multiple hidden onChange={(e) => { addFiles(Array.from(e.target.files ?? [])); e.target.value = ""; }} />
      <AttachmentList files={files} disabled={busy || reading} onRemove={attachments.remove} />
      {delivery && <div role="status" className="mb-1 text-[12px] text-ink-3">{delivery === "sending" ? "Sending…" : delivery === "delivered" ? "Delivered" : delivery === "failed" ? "Attempt failed. Check the trace before starting a new attempt." : "Delivery not confirmed. Retry the same message to check its result."}</div>}
      {delivery === "uncertain" && <button className="btn btn-sm mb-1" disabled={busy} onClick={() => void abandonOperation(`/api/sessions/${sessionId}/input`, { text: message, files }).then(() => setDelivery(null)).catch(toastError)}>I checked the trace — start a new attempt</button>}
      {reading && <div className="mb-1 text-[12px] text-ink-3">Reading attachments…</div>}
      {!ask && <PresetChips selected={presets} onChange={setPresets} disabled={busy} className="mb-1.5" />}
      <div className="session-message-input flex items-end gap-2 rounded-lg border border-line px-2.5 py-[7px] focus-within:border-teal focus-within:shadow-[0_0_0_3px_var(--color-teal-soft)]">
        <textarea
          ref={ref}
          rows={1}
          value={text}
          disabled={disabled || busy || reading}
          onPaste={(e) => {
            if (!acceptsImages || disabled || busy || readingRef.current) return;
            const pasted = pastedImages(e);
            if (pasted) addFiles(pasted);
          }}
          placeholder={placeholder}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            // Enter sends, Shift+Enter adds a new line, like in the terminal.
            if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault();
              void send();
            }
          }}
          className="max-h-40 min-h-5 min-w-0 flex-1 resize-none bg-transparent text-[13px] leading-5 outline-none disabled:opacity-60"
        />
        {!ask && <PresetButton selected={presets} onChange={setPresets} disabled={disabled || busy || reading} />}
        <button className="iconbtn" title={`Attach files or images (you can also drop files${acceptsImages ? " or paste a screenshot" : ""}). ${ATTACH_LIMITS}`} aria-label="Attach files or images" disabled={disabled || busy || reading} onClick={() => fileRef.current?.click()}>
          <Icon name="clip" size={14} />
        </button>
        <button className="iconbtn border-line! bg-paper" title={delivery === "failed" ? "Send as a new attempt" : delivery === "uncertain" ? "Retry delivery" : "Send"} disabled={disabled || busy || reading || (!message && !files.length)} onClick={() => void send()}>
          <Icon name="send" size={14} />
        </button>
      </div>
    </div>
  );
}

/** File paths in the conversation open the code viewer on the session's folder. */
function useCodeLinks(s: { id: string; project_id: string | null; cwd: string } | null): CodeLinks | null {
  const navigate = useNavigate();
  const files = useCodeFiles(s?.project_id ? s.cwd : null, null).data;
  const project = useProjects().data?.find((p) => p.id === s?.project_id);
  return useMemo(() => {
    if (!s?.project_id) return null;
    const { id, project_id, cwd } = s;
    const commitRoot = project?.path ?? cwd;
    return {
      resolve: pathResolver(files?.files ?? [], cwd),
      citation: (raw) => {
        const path = citationPath(raw, cwd);
        if (!path) return null;
        return {
          path, line: null,
          href: `/projects/${encodeURIComponent(project_id)}/code?root=${encodeURIComponent(cwd)}&path=${encodeURIComponent(path)}&session=${encodeURIComponent(id)}`,
          downloadHref: daemonAsset(downloadUrl(cwd, null, path)),
        };
      },
      commits: {
        root: commitRoot,
        href: (sha) => `/projects/${encodeURIComponent(project_id)}/code?root=${encodeURIComponent(commitRoot)}&rev=${sha}&view=diff&session=${encodeURIComponent(id)}`,
        open: (sha) => void navigate({ to: "/projects/$id/code", params: { id: project_id }, search: { root: commitRoot, rev: sha, view: "diff", session: id } }),
      },
      open: (path, line) => void navigate({ to: "/projects/$id/code", params: { id: project_id }, search: { root: cwd, path, line: line ?? undefined, session: id } }),
    };
  }, [files, project?.path, s?.id, s?.project_id, s?.cwd, navigate]);
}

function SessionWorktreeDetails({ session }: { session: Session }) {
  const list = useSessionWorktrees(session.id).data ?? [];
  return <>{list.map((info) => <WorktreeCard key={info.worktree.id} info={info} session={session} />)}</>;
}

type View = "summary" | "trace" | "changes" | "subagents" | "processes";

/** `chat`: an Ask Launchpad thread. It opens on Trace and, on phones, replaces the session header with one status row. */
export function SessionPane({ sessionId, onClear, initialView, chat = false }: { sessionId: string; onClear: () => void; initialView?: "changes" | "trace"; chat?: boolean }) {
  const phone = usePhone();
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [renaming, setRenaming] = useState(false);
  const [title, setTitle] = useState("");
  const [savingTitle, setSavingTitle] = useState(false);
  const { pinned, togglePin, setDock } = useLayout();
  const openTerminal = useOpenTerminal();
  const tui = useTerminals().data?.find((t) => t.alive && t.kind === "tui" && t.session_id === sessionId);
  const [reattaching, setReattaching] = useState(false);
  const dropImages = useRef<((files: File[]) => void) | null>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const [dragging, setDragging] = useState(false);
  const s = useSession(sessionId);
  const processes = useQuery({ queryKey: ["session-processes", sessionId], queryFn: () => api.get<SessionProcess[]>(`/api/sessions/${sessionId}/processes`), enabled: !!s?.live, refetchInterval: s?.live ? 3000 : false, retry: false });
  const runningProcesses = s?.live ? processes.data ?? [] : [];
  const hasProcesses = runningProcesses.length > 0;
  const [gitMain, setGitMain] = useState(false);
  const [view, setView] = useState<View | null>(initialView ?? (chat ? "trace" : null));
  // Running opens Trace when the panel loads or a turn starts; Summary stays selectable while it runs.
  const sessionRunning = !!s && isRunning(s);
  const wasRunning = useRef(sessionRunning);
  useEffect(() => {
    if (s && view === null) setView(sessionRunning ? "trace" : "summary");
    else if (sessionRunning && !wasRunning.current && view === "summary") setView("trace");
    wasRunning.current = sessionRunning;
  }, [s, view, sessionRunning]);
  const git = useQuery({ queryKey: ["session-git", sessionId], queryFn: () => api.get<SessionGitStatus>(`/api/sessions/${sessionId}/git`), refetchInterval: 5000 });
  const history = useQuery({ queryKey: ["session-git-commits", sessionId, ""], queryFn: () => api.get<SessionGitHistory | null>(`/api/sessions/${sessionId}/git-commits`), refetchInterval: 5000 });
  const isGit = !!git.data || !!history.data;
  const current: View = view === "processes" && !hasProcesses ? "trace" : view === "changes" && !isGit ? "summary" : view ?? "summary";
  // Running opens Trace by default (decision 10 allows Summary while running, with a "Working…" band).
  const summary = !s || current === "summary";
  const changesOpen = current === "changes";
  const processesOpen = current === "processes";
  const localPr = useQuery({ queryKey: ["predrafts"], queryFn: () => api.get<import("@/gen/Predraft").Predraft[]>("/api/predrafts") }).data?.find((p) => p.root === s?.cwd || p.id === localStorage.getItem(`launchpad.session-predraft.${sessionId}`));
  const projects = useProjects().data;
  const profileName = useProfile(s?.launch_spec.profile)?.name;
  const navigate = useNavigate();
  const codeLinks = useCodeLinks(s);
  const log = useSessionLogs((st) => st.logs[sessionId]);
  const load = useSessionLogs((st) => st.load);
  const loadEarlier = useSessionLogs((st) => st.loadEarlier);
  const [loadingEarlier, setLoadingEarlier] = useState(false);
  useEffect(() => {
    if (!log) {
      useSessionLogs.setState((st) => ({ logs: { ...st.logs, [sessionId]: emptyLog() } }));
      void load(sessionId);
    }
  }, [sessionId, log, load]);
  const rows = useMemo(() => (log ? buildRows(log) : []), [log]);
  const children = useMemo(() => subagents(log, s?.live ?? false), [log, s?.live]);
  const childrenOpen = current === "subagents";
  const typed = s?.launch_spec.text ?? "";
  const presets = usePresets();
  const trace = useMemo(() => foldTrace(rows, typed, presets), [rows, typed, presets]);
  const ask: PendingAsk = useMemo(() => {
    for (let i = rows.length - 1; i >= 0; i--) {
      const r = rows[i];
      if (r.t === "ask" && !r.answers) return { request_id: r.request_id, count: r.questions.length, permission: r.questions.some((q) => q.permission) };
    }
    const earlier = log?.earlier?.last_ask?.body;
    return earlier?.kind === "ask_user" && !log?.earlier?.last_ask_answers ? { request_id: earlier.request_id, count: earlier.questions.length, permission: earlier.questions.some((q) => q.permission) } : null;
  }, [rows, log?.earlier]);

  const proposals = useProposalList(s?.launch_spec.target.kind === "assistant");
  const threadProposals = (proposals.data ?? []).filter((p) => p.session_id === sessionId).sort((a, b) => a.created_at - b.created_at);
  const rowCount = trace.length + threadProposals.length;
  const scrollRef = useRef<HTMLDivElement>(null);
  const atBottom = useRef(true);
  const traceOpen = !summary && !changesOpen && !childrenOpen && !processesOpen;
  const v = useVirtualizer({ enabled: traceOpen, count: rowCount, getItemKey: (i) => i < trace.length ? trace[i].key : threadProposals[i - trace.length].id, getScrollElement: () => scrollRef.current, estimateSize: () => 48, overscan: 6, useFlushSync: false });
  const total = v.getTotalSize();
  useLayoutEffect(() => {
    if (atBottom.current && rowCount) v.scrollToIndex(rowCount - 1, { align: "end" });
  }, [rowCount, total, v, summary, changesOpen]);
  // Answer: show the open question in the trace and focus the message box.
  const [answering, setAnswering] = useState(0);
  useEffect(() => {
    if (!answering || !traceOpen) return;
    const i = trace.findLastIndex((r) => r.t === "ask" && !r.answers);
    atBottom.current = false;
    if (i >= 0) v.scrollToIndex(i, { align: "start" });
    inputRef.current?.focus();
    setAnswering(0);
  }, [answering, traceOpen]);
  // Re-render every 20 s for elapsed times.
  const [, tick] = useState(0);
  useEffect(() => {
    const t = setInterval(() => tick((n) => n + 1), 20_000);
    return () => clearInterval(t);
  }, []);

  const harness = useHarnessInfo(s?.harness ?? null);
  if (!s) {
    return (
      <div className="grid h-full place-items-center text-ink-3">
        This session no longer exists.
      </div>
    );
  }
  const running = isRunning(s);
  const resumable = s.status === "closed" || s.status === "stopped" || s.status === "interrupted" || s.status === "error";
  const name = harness?.name ?? s.harness;
  const isPinned = pinned.includes(s.id);
  const mainChanges = git.data?.main_changes?.files ?? 0;
  // Committed differences from main remain reviewable, but do not block Archive.
  const uncommitted = (git.data?.files.length ?? 0) > 0;
  const committedFiles = history.data?.files.length ?? 0;
  const changed = uncommitted || mainChanges > 0 || committedFiles > 0;
  const showChanges = (main: boolean) => { setGitMain(main); setView("changes"); };
  const onlyMain = (git.data?.files.length ?? 0) === 0 && mainChanges > 0;
  const archive = () => void act(s.id, "close");
  const placeholder =
    s.status === "in_terminal"
      ? "Open in the terminal: re-attach it to continue here"
      : s.open_question
        ? `Answer or message ${name}`
        : running
          ? s.status === "background" ? `Message ${name}. Background work is still running.` : harness?.working_message ?? `Message ${name}`
          : resumable
            ? `Message ${name} to resume the session`
            : `Message ${name}`;

  // The one primary action, depending on the state.
  const primary = running || s.status === "quota_paused" ? (
    <button className="btn btn-sm border-gh-closed! bg-danger! text-white!" title={s.status === "quota_paused" ? "Stop: cancel automatic continuation" : "Stop: interrupt the agent"} onClick={() => void act(s.id, "interrupt")}>
      <Icon name="stop" size={12} />Stop
    </button>
  ) : s.status === "in_terminal" ? (
    tui && reattaching ? <>
      <button className="btn btn-sm btn-primary" onClick={() => { setReattaching(false); void api.post(`/api/sessions/${s.id}/resume`, { close_terminal: true }).catch(toastError); }}>Close the TUI and re-attach</button>
      <button className="btn btn-sm" onClick={() => setReattaching(false)}>Cancel</button>
    </> : <button className="btn btn-sm btn-primary" title="Continue this session in Launchpad" onClick={() => (tui ? setReattaching(true) : void act(s.id, "resume"))}>Re-attach</button>
  ) : s.status === "waiting" && s.open_question ? (
    <>
      <button className="btn btn-sm btn-primary" title={ask?.permission ? "Review the requested operation" : "Show the question and focus the message box"} onClick={() => { setView("trace"); setAnswering((n) => n + 1); }}>{ask?.permission ? "Review approval" : "Answer"}</button>
      {s.live && <button className="iconbtn text-danger" title="Stop the agent while it waits for your decision" aria-label="Stop agent" onClick={() => void act(s.id, "interrupt")}><Icon name="stop" size={14} /></button>}
    </>
  ) : s.status === "waiting" ? (
    uncommitted
      ? <button className="btn btn-sm btn-primary" title="Open the uncommitted files in the Changes view" onClick={() => showChanges(false)}>Review changes</button>
      : <button className="btn btn-sm btn-primary" title="Archive the session (it stays resumable)" onClick={archive}><Icon name="archive" size={12} />Archive</button>
  ) : resumable ? (
    <button className="btn btn-sm btn-primary" title={s.status === "closed" ? "Re-attach this archived session; send its original task if it never started" : "Continue the unfinished task"} onClick={() => { void (s.status === "closed" ? act(s.id, "resume") : api.post(`/api/sessions/${s.id}/resume`, { continue_turn: true }).catch(toastError)); setView("trace"); }}>Resume</button>
  ) : null;
  const archivePrimary = s.status === "waiting" && !s.open_question && !uncommitted;

  return (
    <div className="relative flex h-full min-h-0 min-w-0 flex-col bg-paper"
      onDragOver={(e) => { if (e.dataTransfer.types.includes("Files")) { e.preventDefault(); setDragging(true); } }}
      onDragLeave={(e) => { if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setDragging(false); }}
      onDrop={(e) => { if (e.dataTransfer.types.includes("Files")) { e.preventDefault(); setDragging(false); dropImages.current?.(Array.from(e.dataTransfer.files)); } }}
    >
      {dragging && <div className="pointer-events-none absolute inset-0 z-10 grid place-items-center border-2 border-teal bg-teal-soft/80 font-semibold text-teal-ink">{harness?.capabilities.images ? "Drop files or images to attach to your message" : `Drop files to attach (${name} does not accept images)`}</div>}
      {phone && chat ? <div className="flex min-h-11 items-center gap-2 border-b border-line-2 px-3 py-1 text-[12px] text-ink-3">
        <StatusBadge s={s} />
        <span className="min-w-0 flex-1 truncate">{usageLine(s)}</span>
        {!archivePrimary && primary}
        <button className="btn btn-sm" aria-pressed={summary} title="The thread's run facts, model and effort" onClick={() => setView(summary ? "trace" : "summary")}>{summary ? "Chat" : "Summary"}</button>
      </div> : <div className="session-header grid grid-cols-[18px_1fr] items-center gap-x-[9px] gap-y-0.5 border-b border-line-2 py-2 pr-2.5 pl-3">
        <span className={`order-1 row-span-2 self-start pt-px ${phone ? "hidden" : ""}`}>
          <HarnessMark harness={s.harness} size={18} />
        </span>
        <div className="session-heading order-2 flex min-w-0 items-center gap-1">
          {renaming ? <div className="flex min-w-0 flex-1 flex-wrap items-center gap-1">
            <input autoFocus aria-label="Session title" className="field-input min-w-0 flex-1 basis-32 text-[13px] font-[650]" maxLength={100} value={title} disabled={savingTitle} onChange={(e) => setTitle(e.target.value)} />
            <button className="btn btn-sm" disabled={!title.trim() || savingTitle} onClick={async () => { setSavingTitle(true); try { await api.post(`/api/sessions/${s.id}/rename`, { title: title.trim() }); setRenaming(false); } catch (e) { toastError(e); } finally { setSavingTitle(false); } }}>Save title</button>
            <button className="btn btn-sm" disabled={savingTitle} onClick={() => setRenaming(false)}>Cancel</button>
          </div> : <button className="session-title min-w-0 flex-1 truncate text-left text-[13px] font-[650] hover:text-teal-ink" title={`${s.title} · Click to edit`} aria-label={`Edit session title: ${s.title}`} onClick={() => { setTitle(s.title); setRenaming(true); }}>
            {isPinned && <span className="mr-1 text-teal-ink" title="Pinned">★</span>}{s.title}
          </button>}
          {phone
            ? <button className="iconbtn session-back order-first shrink-0 self-start" title="Back to agents" aria-label="Back to agents" onClick={onClear}><Icon name="chev" size={20} className="rotate-90" /></button>
            : <button className="iconbtn shrink-0 self-start" title="Close panel" aria-label="Close panel" onClick={onClear}>
              <Icon name="x" size={14} />
            </button>}
        </div>
        <span className="session-meta order-3 col-start-2 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-[12px] text-ink-3 tabular-nums">
          {phone ? <>
            <HostLabel path={s.cwd} />
            <StatusBadge s={s} />
            <button className="session-details min-w-0 flex-1 truncate text-left" aria-label="Session details" onClick={() => setDetailsOpen(true)}>{sessionPlace(s, projectName(projects, s.project_id, s.cwd))} <Icon name="chev" size={12} /></button>
            <span className="ml-auto flex flex-wrap justify-end gap-1">{primary}</span>
            <RunSettingsChip session={s} open={() => setView("summary")} />
            <PaidUsageToggle session={s} badge />
            <CostLimit session={s} badge />
          </> : <>
            <StatusBadge s={s} />
            <span className="truncate" title={`Started ${new Date(s.started_at).toLocaleString()}`}>{sessionPlace(s, projectName(projects, s.project_id, s.cwd))}</span>
            <HostLabel path={s.cwd} />
            <WorktreeChip session={s} />
            {localPr && <button className="text-teal-ink hover:underline" onClick={() => void navigate({ to: "/predrafts/$id", params: { id: localPr.id } })}>Local PR</button>}
            {s.launch_spec.profile && <span title="Profile">{profileName ?? `${s.launch_spec.profile} (deleted profile)`}</span>}
            {usageLine(s) && <span>{usageLine(s)}</span>}
            <RunSettingsChip session={s} open={() => setView("summary")} />
            <PaidUsageToggle session={s} badge />
            <CostLimit session={s} badge />
          </>}
        </span>
        <div className="session-toolbar order-4 col-span-2 flex flex-wrap items-center gap-1 pt-1">
          <Tabs tour="session.views" className="session-views mr-1 max-w-full border-b-0" label="Session view" value={summary ? "summary" : changesOpen ? "changes" : processesOpen ? "processes" : childrenOpen ? "subagents" : "trace"} onChange={(v) => (v === "changes" ? showChanges(onlyMain) : setView(v))} items={[
            { id: "summary", label: "Summary" },
            { id: "trace", label: "Trace" },
            ...(hasProcesses ? [{ id: "processes" as const, label: `Processes · ${runningProcesses.length}` }] : []),
            ...(children.length > 0 ? [{ id: "subagents" as const, icon: "agents" as const, label: `Subagents · ${children.filter((a) => a.status === "running").length || children.length}${children.some((a) => a.status === "running") ? " running" : ""}` }] : []),
            ...(isGit ? [{ id: "changes" as const, title: "Git changes of the session folder", label: `Changes${changed ? ` · ${git.data?.files.length || committedFiles || mainChanges}` : ""}` }] : []),
          ]} />
          <div className="session-actions ml-auto flex flex-wrap items-center gap-1">
            {!phone && <>
              <span data-tour="session.primary" className="inline-flex">{primary}</span>
              <FollowUpAgent session={s} />
              {!(s.launch_spec.target.kind === "browser" && !s.launch_spec.target.project_id) && <TerminalMenu cwd={s.cwd} small />}
            </>}
            <Popover viewport={phone} label={phone ? "Session actions" : undefined} align="right" width={phone ? 280 : 260} trigger={({ open, toggle }) => <button className={phone ? "iconbtn" : "btn btn-sm"} aria-label="Session actions" title="Session actions" aria-haspopup={phone ? "dialog" : undefined} aria-expanded={open} onClick={toggle}>{phone ? <Icon name="more" size={20} /> : <>More<Icon name="chev" size={12} /></>}</button>}>
              {(close) => <>
                {isDone(s) && <MenuItem icon={<Icon name="mail" size={16} />} label="Mark as unread" detail="Return to Needs you for later" onClick={() => { close(); onClear(); void act(s.id, "unread"); }} />}
                {phone && <>
                  <MenuItem label="Session details" detail="Branch, usage and New agent here" onClick={() => { close(); setDetailsOpen(true); }} />
                  {!(s.launch_spec.target.kind === "browser" && !s.launch_spec.target.project_id) && <MenuItem icon={<Icon name="term" size={16} />} label="Terminal" detail="Open a shell in this folder (the checkout when its worktree is gone)" onClick={() => { close(); void openTerminal.folder(s.cwd); }} />}
                </>}
                {s.status === "in_terminal" && tui && <MenuItem icon={<Icon name="term" size={14} />} label="Show TUI tab" detail="In the terminal dock" onClick={() => { close(); setDock({ open: true, active: tui.id }); }} />}
                {s.launch_spec.target.kind !== "browser" && !running && s.status !== "in_terminal" && s.status !== "quota_paused" && harness?.capabilities.terminal && <MenuItem icon={<Icon name="term" size={14} />} label={`Continue in ${name} TUI`} detail="In the terminal dock, outside quota protection; Re-attach brings it back" onClick={() => { close(); void openTerminal.tui(s.id); }} />}
                {s.project_id && <MenuItem icon={<Icon name="folder" size={14} />} label="Browse files" detail="Read the folder in Files" onClick={() => { close(); void navigate({ to: "/projects/$id/code", params: { id: s.project_id! }, search: { root: s.cwd, session: s.id } }); }} />}
                <MenuItem icon={<Icon name="agents" size={14} />} label="Use trace as context" detail="Reference this trace in a new agent's prompt" disabled={s.last_seq === 0} onClick={() => { close(); useComposer.getState().start({ agentTraces: [{ session_id: s.id, through_seq: s.last_seq }] }); }} />
                {s.status !== "closed" && !archivePrimary && <MenuItem icon={<Icon name="archive" size={14} />} label="Archive" detail="It stays resumable" onClick={() => { close(); archive(); }} />}
                <MenuItem icon={<span className="text-teal-ink">{isPinned ? "★" : "☆"}</span>} label={isPinned ? "Unpin session" : "Pin session"} onClick={() => { close(); togglePin(s.id); }} />
              </>}
            </Popover>
          </div>
        </div>
      </div>}
      {phone && detailsOpen && <Sheet title="Session details" onClose={() => setDetailsOpen(false)}>
        <div className="session-detail-sheet grid gap-4 p-4 text-sm">
          <div className="flex items-start gap-2"><HarnessMark harness={s.harness} size={20} /><div className="min-w-0"><b className="break-words">{s.title}</b><p className="mt-1 text-ink-3">{sessionPlace(s, projectName(projects, s.project_id, s.cwd))} · {s.launch_spec.profile ? profileName ?? `${s.launch_spec.profile} (deleted profile)` : name}</p></div></div>
          <dl className="grid gap-2 text-ink-2">
            {usageLine(s) && <div><dt className="text-xs text-ink-3">Usage</dt><dd>{usageLine(s)}</dd></div>}
            <div><dt className="text-xs text-ink-3">Started</dt><dd>{new Date(s.started_at).toLocaleString()}</dd></div>
          </dl>
          {localPr && <button className="btn" onClick={() => { setDetailsOpen(false); void navigate({ to: "/predrafts/$id", params: { id: localPr.id } }); }}>Local PR</button>}
          <SessionWorktreeDetails session={s} />
          <FollowUpAgent session={s} onPrepared={() => setDetailsOpen(false)} />
        </div>
      </Sheet>}

      {log?.error && <div role="alert" className="flex flex-wrap items-center gap-3 px-4 py-3 text-ink-3">
        <span className="min-w-0 flex-1 break-words">Could not load session events: {log.error}</span>
        <button className="btn btn-sm" disabled={log.loading} onClick={() => void load(sessionId)}>{log.loading ? "Loading…" : "Retry"}</button>
      </div>}
      <HostBand cwd={s.cwd} running={running} />
      {s.capacity_retry_at != null && <div role="status" className="flex flex-wrap items-center gap-2 border-b border-line bg-teal-soft px-3 py-2 text-xs text-teal-ink">
        <Icon name="clock" size={16} />Model at capacity · automatic retry {s.capacity_retry_count + 1} scheduled for {new Date(s.capacity_retry_at).toLocaleTimeString()}. Resume now or archive to cancel.
      </div>}
      {s.launch_spec.target.kind === "browser" && <div className="border-b border-line bg-teal-soft px-3 py-1 text-xs text-teal-ink">Browser task · tabs shared from the Chrome extension · {s.launch_spec.target.act ? "Act" : "Read"}</div>}
      {s.live && processes.isError && <div role="alert" className="px-3 py-2 text-xs text-danger">{String(processes.error)} <button className="btn btn-sm" onClick={() => void processes.refetch()}>Retry</button></div>}
      {hasProcesses && !processesOpen && !running && <div className="flex items-center gap-2 px-3 py-2 text-xs text-ink-3">Background commands are still running.<button className="btn btn-sm" onClick={() => setView("processes")}>View processes</button></div>}
      {processesOpen && <ProcessesPanel sessionId={s.id} processes={runningProcesses} refresh={() => processes.refetch()} />}
      {changesOpen && <GitPanel key={`${s.id}-${gitMain}`} initialMain={gitMain} sessionId={s.id} canMessage={s.status !== "in_terminal" && s.status !== "quota_paused" && !(running && !harness?.capabilities.messages_while_running)} />}
      <CodeLinksContext.Provider value={codeLinks}>
      <ShellCodeContext.Provider value={s.launch_spec.target.kind === "browser" && !s.launch_spec.target.project_id ? null : s.cwd}>
      {childrenOpen && log && <SubagentsPanel key={s.id} log={log} agents={children} sessionId={s.id} live={s.live} back={() => setView("trace")} />}
      {!changesOpen && !childrenOpen && !processesOpen && summary && (log && !log.loaded ? <div className="flex-1 p-6 text-center text-ink-3">{log.error ? "Session events unavailable." : "Loading…"}</div> : <SessionSummary key={s.id} session={s} rows={rows} earlier={log?.earlier ?? null} localPr={localPr} reviewDiff={showChanges} showTrace={() => setView("trace")} />)}
      {traceOpen && !running && <>{!(phone && chat) && <StateBand session={s}>
        <button className="btn btn-sm" disabled={!rows.some((r) => r.t === "assistant")} onClick={() => { atBottom.current = false; const i = trace.findLastIndex((r) => r.t === "assistant"); if (i >= 0) v.scrollToIndex(i, { align: "start" }); }}>Final message</button>
        <button className="btn btn-sm" onClick={() => setView("summary")}>Summary</button>
      </StateBand>}<FollowUpBands sessionId={s.id} /><QuotaBand session={s} /></>}
      <div
        ref={scrollRef}
        className={`min-h-0 flex-1 overflow-y-auto px-3 ${traceOpen ? "" : "hidden"}`}
        onScroll={(e) => {
          const el = e.currentTarget;
          atBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < 60;
        }}
      >
        {log?.loaded && log.earlier && (
          <div className="flex justify-center pt-2">
            <button
              className="btn btn-sm"
              disabled={loadingEarlier}
              title="Earlier events of this session are not loaded yet"
              onClick={() => {
                const before = trace.length;
                setLoadingEarlier(true);
                atBottom.current = false;
                void loadEarlier(s.id)
                  .then(() => requestAnimationFrame(() => {
                    // Keep the first previously shown row in view.
                    const added = foldTrace(buildRows(useSessionLogs.getState().logs[s.id]), typed, presets).length - before;
                    if (added > 0) v.scrollToIndex(added, { align: "start" });
                  }))
                  .catch(toastError)
                  .finally(() => setLoadingEarlier(false));
              }}
            >
              {loadingEarlier ? "Loading…" : "Load earlier events"}
            </button>
          </div>
        )}
        {log && !log.loaded ? (
          <div className="p-6 text-center text-ink-3">Loading…</div>
        ) : (
          <div style={{ height: total, position: "relative" }}>
            {v.getVirtualItems().map((it) => (
              <div
                key={it.key}
                data-index={it.index}
                ref={v.measureElement}
                className="flex flex-col py-[5px]"
                style={{ position: "absolute", top: 0, left: 0, right: 0, transform: `translateY(${it.start}px)` }}
              >
                {it.index < trace.length ? <EventRow row={trace[it.index]} sessionId={s.id} live={s.live} /> : <ProposalCard p={threadProposals[it.index - trace.length]} />}
              </div>
            ))}
          </div>
        )}
        {running && (
          <div className="flex items-center gap-2 pb-2 text-xs text-work">
            <span className="lp-spin" />
            {s.status === "starting" ? "Starting…" : s.status === "background" ? "Background work is still running…" : "Working…"}
          </div>
        )}
      </div>
      </ShellCodeContext.Provider>
      </CodeLinksContext.Provider>
      {!childrenOpen && <MessageBox acceptsImages={!!harness?.capabilities.images} onSent={() => setView("trace")} key={s.id} dropImages={dropImages} inputRef={inputRef} sessionId={s.id} placeholder={s.open_question && ask?.permission ? "Choose a decision on the approval card." : placeholder} disabled={!!(s.open_question && ask?.permission) || s.status === "in_terminal" || s.status === "quota_paused" || (running && !s.open_question && !harness?.capabilities.messages_while_running)} ask={s.open_question && s.live ? ask : null} />}
    </div>
  );
}

/** The host of the session folder is not connected: a connection state, never a session status. */
function HostBand({ cwd, running }: { cwd: string; running: boolean }) {
  const id = hostOf(cwd);
  const h = useHost(id);
  if (!id || !h || h.state === "connected") return null;
  const since = new Date(Number(h.since)).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  return (
    <div className="flex flex-wrap items-center gap-2 border-b border-attn-line bg-attn-soft px-3 py-2 text-[12px]" role="status">
      <span className="min-w-0 flex-1">
        <b>{id} is {linkLabel[h.state].toLowerCase()}{h.state === "connecting" ? "" : ` since ${since}`}.</b>{" "}
        {running ? "The agent keeps running on the host; its output appears here when the connection returns. Stop is sent once it is back." : "Changes, files and new turns need the host; Launchpad reconnects every 15 s."}
        {h.error && <span className="block text-ink-3">{h.error}</span>}
      </span>
      <button className="btn btn-sm" onClick={() => void api.post(`/api/hosts/${id}/retry`).catch(toastError)}>Retry now</button>
    </div>
  );
}
