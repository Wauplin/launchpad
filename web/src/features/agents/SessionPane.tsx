import { SessionTopics } from "@/features/topics/TopicPicker";
// One session in a pane: header with state and actions, the conversation, and the message box.
// The conversation is virtualized (spike 4 rules: memoized rows, no flushSync).

import { when } from "@/lib/format";
import { ProposalCard, useProposalList } from "@/features/proposals/ProposalCard";
import { FollowUpAgent } from "./FollowUpAgent";
import { useProfile } from "@/lib/profiles";
import { SessionSummary, StateBand, QuotaBand, FollowUpBands, CostLimit, PaidUsageToggle, RunSettingsChip } from "./SessionSummary";
import { BotChip, Tabs } from "@/components/ui";
import { SubagentsPanel, subagents } from "./SubagentsPanel";
import { GitPanel } from "./GitPanel";
import { ProcessesPanel } from "./ProcessesPanel";
import type { SessionProcess } from "@/gen/SessionProcess";
import { useComposer } from "@/stores/composer";
import { useLayout } from "@/stores/layout";
import { useQuery } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { Session } from "@/gen/Session";
import { HarnessMark, useHarnessInfo } from "@/components/HarnessMark";
import { CodeLinksContext, ShellCodeContext, type CodeLinks } from "@/components/Markdown";
import { citationPath } from "@/lib/fileCitations";
import { daemonAsset } from "@/lib/connection";
import { downloadUrl, pathResolver, useCodeFiles } from "@/features/code/lib";
import { useMessageDrafts } from "@/stores/messages";
import { useRunners } from "@/lib/runners";
import { isDone, isRunning } from "@/lib/sessionStatus";
import { Icon } from "@/components/Icon";
import { toastError } from "@/components/Toast";
import { api, ApiError } from "@/lib/api";
import { projectName, useProjects, useSession, useSettled } from "@/lib/queries";
import { emptyLog, useSessionLogs } from "@/lib/sessionStore";
import { buildRows } from "./rows";
import { EventRow } from "./EventRow";
import { foldTrace } from "./traceRows";
import { MenuItem, Popover } from "@/components/Popover";
import { ActionMenuItem } from "@/components/ActionButton";
import type { SessionGitHistory } from "@/gen/SessionGitHistory";
import type { SessionGitStatus } from "@/gen/SessionGitStatus";
import { SessionStatus, usageLine } from "./SessionBits";
import { useOpenTerminal, useTerminalBlock, useTerminals, useTerminalsHere } from "@/features/terminals/lib";
import { TerminalMenu } from "@/features/terminals/TerminalMenu";
import { Sheet } from "@/mobile/Sheet";
import { gitPoll, useGitRefreshAtTurnEnd, WorktreeCard, useSessionWorktrees, WorktreeChip } from "./Worktrees";
import { usePhone } from "@/lib/phone";
import { useBackClose } from "@/mobile/back";
import { PhoneBar } from "@/mobile/PhoneHeader";
import { usePresets } from "./MessagePresets";
import { MessageBox, type PendingAsk } from "./MessageBox";
import { LocalPrButtons, LocalPrChip } from "@/features/predrafts/LocalPrLink";
import { runAction } from "@/lib/actions";
import { FixMergeButton, MergeButton, mergeIsMain, NotMerged, useBranch } from "@/features/today/TodayCards";

function act(id: string, action: "session.archive" | "session.stop" | "session.resume" | "session.unread") {
  return runAction(action, { id }).catch(toastError);
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

/** The session's one main action: Merge into the base when it is safe (the same check as Today cards), else the
 * state's action (Answer, Review changes, Archive, Resume, Stop…). In the laptop header and the phone bottom bar. */
function StatePrimary({ s, primary }: { s: Session; primary: React.ReactNode }) {
  const branch = useBranch(s.id, s);
  if (mergeIsMain(s, branch)) return <MergeButton sessionId={s.id} branch={branch} />;
  return <>{primary}</>;
}

export type View = "summary" | "trace" | "changes" | "subagents" | "processes";

/** `initialView`: the view to show (the phone keeps it in the address, `onView` writes it there with a replace). */
export function SessionPane({ sessionId, onClear, initialView, onView }: { sessionId: string; onClear: () => void; initialView?: View; onView?: (view: View) => void }) {
  const phone = usePhone();
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [topicsOpen, setTopicsOpen] = useState(false);
  const [expanded, setExpanded] = useState(false);
  // Phone: Reply opens the message box in place of the bottom bar, so no other action sits under the keyboard;
  // Hide closes it. A session with a draft opens with it.
  const [replying, setReplying] = useState(() => !!useMessageDrafts.getState().drafts[sessionId]);
  useEffect(() => { setExpanded(false); setReplying(!!useMessageDrafts.getState().drafts[sessionId]); }, [sessionId]);
  const [renaming, setRenaming] = useState(false);
  const [title, setTitle] = useState("");
  const [savingTitle, setSavingTitle] = useState(false);
  const { pinned, togglePin, setDock } = useLayout();
  const openTerminal = useOpenTerminal();
  const terminalsHere = useTerminalsHere();
  const tui = useTerminals().data?.find((t) => t.alive && t.kind === "tui" && t.session_id === sessionId);
  const [reattaching, setReattaching] = useState(false);
  const dropImages = useRef<((files: File[]) => void) | null>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const [dragging, setDragging] = useState(false);
  const s = useSession(sessionId);
  const lastProcessesError = useRef<unknown>(null);
  // Terminals open on the agent's runner, relayed by Home: why not now (offline runner, stopped Burst job), or null.
  const terminalBlock = useTerminalBlock(s?.runner_id);
  // Background commands of a live session, polled from its runner; never while that runner is offline (its badge says so).
  const runnerOffline = useRunners().data?.some((r) => r.runner.id === s?.runner_id && !r.connected) ?? false;
  const pollProcesses = !!s?.live && !runnerOffline;
  const processes = useQuery({ queryKey: ["session-processes", sessionId], queryFn: () => api.get<SessionProcess[]>(`/api/sessions/${sessionId}/processes`), enabled: pollProcesses, refetchInterval: pollProcesses ? 3000 : false, retry: false });
  // A refetch after an error without data clears the error until it answers: keep the last one shown so the alert does not blink.
  const processesError = processes.errorUpdatedAt > processes.dataUpdatedAt ? (processes.error ?? processes.failureReason ?? lastProcessesError.current) : null;
  if (processesError) lastProcessesError.current = processesError;
  const runningProcesses = s?.live ? processes.data ?? [] : [];
  const hasProcesses = runningProcesses.length > 0;
  const [gitMain, setGitMain] = useState(false);
  const [view, setViewState] = useState<View | null>(initialView ?? null);
  const setView = (v: View) => { setViewState(v); onView?.(v); };
  // The address changed the view (a link, or back to an entry that showed another view).
  useEffect(() => { if (initialView) setViewState(initialView); }, [initialView]);
  // Running opens Trace when the panel loads or a turn starts; Summary stays selectable while it runs.
  const sessionRunning = !!s && isRunning(s);
  const wasRunning = useRef(sessionRunning);
  useEffect(() => {
    if (s && view === null) setViewState(sessionRunning ? "trace" : "summary");
    else if (sessionRunning && !wasRunning.current && view === "summary") setViewState("trace");
    wasRunning.current = sessionRunning;
  }, [s, view, sessionRunning]);
  const git = useQuery({ queryKey: ["session-git", sessionId], queryFn: () => api.get<SessionGitStatus>(`/api/sessions/${sessionId}/git`), refetchInterval: gitPoll(s) });
  const history = useQuery({ queryKey: ["session-git-commits", sessionId, ""], queryFn: () => api.get<SessionGitHistory | null>(`/api/sessions/${sessionId}/git-commits`), refetchInterval: gitPoll(s) });
  useGitRefreshAtTurnEnd(s);
  // Settled outcomes: a failed Git query (a folder that is not Git) goes back to pending at each poll, which made Archive blink.
  const gitState = useSettled(git);
  const historyState = useSettled(history);
  const isGit = !!git.data || !!history.data;
  const current: View = view === "processes" && !hasProcesses ? "trace" : view === "changes" && !isGit ? "summary" : view ?? "summary";
  // Running opens Trace by default (decision 10 allows Summary while running, with a "Working…" band).
  const summary = !s || current === "summary";
  const canExpand = phone && (current === "summary" || current === "trace");
  const readingExpanded = canExpand && expanded;
  // Phone: the full-screen reading mode and the Reply box are overlays; Android back closes them (the draft stays).
  useBackClose(readingExpanded, () => setExpanded(false));
  useBackClose(phone && replying, () => setReplying(false));
  const changesOpen = current === "changes";
  const processesOpen = current === "processes";
  // The Local PR this window created for the session (remembered at creation), read once per render, not per row.
  const rememberedPr = localStorage.getItem(`launchpad.session-predraft.${sessionId}`);
  const localPr = useQuery({ queryKey: ["predrafts"], queryFn: () => api.get<import("@/gen/Predraft").Predraft[]>("/api/predrafts") }).data?.find((p) => !p.archived && (p.session_id === sessionId || p.id === rememberedPr));
  // A worktree card (branch popover, details sheet, Next steps) already opens the Local PR of its branch.
  const sessionWorktrees = useSessionWorktrees(sessionId).data;
  const prInCard = !!localPr && !!sessionWorktrees?.some((i) => i.worktree.branch === localPr.branch && i.worktree.state !== "discarded");
  const projects = useProjects().data;
  const branch = useBranch(sessionId, s ?? null);
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

  // Cards this session wrote (`launchpad propose`) follow its trace.
  const proposals = useProposalList();
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
  // ⋯ → Jump to the final message.
  const [jumpFinal, setJumpFinal] = useState(0);
  useEffect(() => {
    if (!jumpFinal || !traceOpen) return;
    const i = trace.findLastIndex((r) => r.t === "assistant");
    atBottom.current = false;
    if (i >= 0) v.scrollToIndex(i, { align: "start" });
    setJumpFinal(0);
  }, [jumpFinal, traceOpen]);
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
  // Committed work already merged, in a Local PR or in a GitHub pull request stays reviewable in Changes, but
  // Archive becomes the main action.
  const handedOff = !uncommitted && (branch.handedOff || !!history.data?.merged_into || !!localPr);
  const toReview = changed && !handedOff;
  // A merge that stopped (conflict or failure): Fix merge comes before Review changes and Archive.
  const mergeStopped = s.status === "waiting" && !s.open_question && !!branch.stopped;
  // A Git query that failed (its runner is away, a network error) means unknown, never "no changes": Archive
  // becomes the primary action only once Git confirmed there is nothing to review. A 4xx answer for a folder
  // that is not a Home Git folder means it is not a Git folder.
  const gitFolder = !!projects?.find((p) => p.id === s.project_id)?.home_git;
  const failed = (e: unknown) => !(e instanceof ApiError) || e.status >= 500 || gitFolder;
  const gitLoading = gitState.pending || historyState.pending;
  const gitUnknown = !changed && !handedOff && !gitLoading && (historyState.error != null || (gitState.error != null && failed(gitState.error)));
  const showChanges = (main: boolean) => { setGitMain(main); setView("changes"); };
  const onlyMain = (git.data?.files.length ?? 0) === 0 && mainChanges > 0;
  const archive = () => void act(s.id, "session.archive");
  const quotaPaused = s.status === "quota_paused";
  const quotaMessageBlocked = quotaPaused && !["pausing", "paused", "provider_limited"].includes(s.quota_pause?.phase ?? "");
  const placeholder = quotaPaused
    ? quotaMessageBlocked ? "Wait for resumption or inspect the quota pause before messaging" : "Queue a message for when the agent resumes"
    : s.status === "in_terminal"
      ? "Open in the terminal: re-attach it to continue here"
      : s.open_question
        ? ask && ask.count > 1 ? `Answer the ${ask.count} questions: one line each, in order` : `Answer or message ${name}`
        : running
          ? s.status === "background" ? `Message ${name}. Background work is still running.` : harness?.working_message ?? `Message ${name}`
          : resumable
            ? `Message ${name} to resume the session`
            : `Message ${name}`;

  // The one primary action, depending on the state.
  const primary = running || s.status === "quota_paused" ? (
    <button className="btn btn-sm btn-danger" title={s.status === "quota_paused" ? "Stop: cancel automatic continuation" : "Stop: interrupt the agent"} onClick={() => void act(s.id, "session.stop")}>
      <Icon name="stop" size={12} />Stop
    </button>
  ) : s.status === "in_terminal" ? (
    tui && reattaching ? <>
      <button className="btn btn-sm btn-primary" onClick={() => { setReattaching(false); void api.post(`/api/sessions/${s.id}/resume`, { close_terminal: true }).catch(toastError); }}>Close the TUI and re-attach</button>
      <button className="btn btn-sm" onClick={() => setReattaching(false)}>Cancel</button>
    </> : <button className="btn btn-sm btn-primary" title="Continue this session in Launchpad" onClick={() => (tui ? setReattaching(true) : void act(s.id, "session.resume"))}>Re-attach</button>
  ) : s.status === "waiting" && s.open_question ? (
    <>
      <button className="btn btn-sm btn-primary" title={ask?.permission ? "Review the requested operation" : "Show the question and focus the message box"} onClick={() => { setExpanded(false); setReplying(true); setView("trace"); setAnswering((n) => n + 1); }}>{ask?.permission ? "Review approval" : "Answer"}</button>
      {s.live && !phone && <button className="iconbtn text-danger" title="Stop the agent while it waits for your decision" aria-label="Stop agent" onClick={() => void act(s.id, "session.stop")}><Icon name="stop" size={14} /></button>}
    </>
  ) : mergeStopped ? (
    <FixMergeButton sessionId={s.id} branch={branch} />
  ) : s.status === "waiting" ? (
    // Work not handed off yet (uncommitted, or commits not merged and in no Local PR or pull request) comes before Archive.
    toReview
      ? <button className="btn btn-sm btn-primary" title={uncommitted ? "Open the uncommitted files in the Changes view" : "Open the agent's changes: merge, open a Local PR or keep the branch from there"} onClick={() => showChanges(!uncommitted && mainChanges > 0)}>Review changes</button>
      : gitUnknown
        ? <button className="btn btn-sm btn-primary" title="Git state unknown: check the worktree and its branch before archiving" onClick={() => setView("summary")}>Review work</button>
        : gitLoading ? null : <button className="btn btn-sm btn-primary" title="Archive the session (it stays resumable)" onClick={archive}><Icon name="archive" size={12} />Archive</button>
  ) : resumable ? (
    <button className="btn btn-sm btn-primary" title={s.status === "closed" ? "Re-attach this archived session; send its original task if it never started" : "Continue the unfinished task"} onClick={() => { void (s.status === "closed" ? act(s.id, "session.resume") : api.post(`/api/sessions/${s.id}/resume`, { continue_turn: true }).catch(toastError)); setView("trace"); }}>Resume</button>
  ) : null;
  const archivePrimary = s.status === "waiting" && !s.open_question && !mergeStopped && !toReview && !gitUnknown && !gitLoading;
  const canReply = !childrenOpen;

  const place = projectName(projects, s.project_id, s.cwd);
  const profileLabel = s.launch_spec.profile ? profileName ?? `${s.launch_spec.profile} (deleted profile)` : null;
  const contextShare = s.usage.context_tokens && s.usage.context_window ? `${Math.round((100 * s.usage.context_tokens) / s.usage.context_window)}% context` : null;
  const titleEl = renaming ? <div className="flex min-w-0 flex-1 flex-wrap items-center gap-1">
    <input autoFocus aria-label="Session title" className="field-input min-w-0 flex-1 basis-32 font-[650]" maxLength={100} value={title} disabled={savingTitle} onChange={(e) => setTitle(e.target.value)} />
    <button className="btn btn-sm" disabled={!title.trim() || savingTitle} onClick={async () => { setSavingTitle(true); try { await api.post(`/api/sessions/${s.id}/rename`, { title: title.trim() }); setRenaming(false); } catch (e) { toastError(e); } finally { setSavingTitle(false); } }}>Save title</button>
    <button className="btn btn-sm" disabled={savingTitle} onClick={() => setRenaming(false)}>Cancel</button>
  </div> : <button className={`session-title min-w-0 truncate text-left text-[13px] font-[650] hover:text-teal-ink ${phone ? "flex-1" : ""}`} title={`${s.title} · Click to edit`} aria-label={`Edit session title: ${s.title}`} onClick={() => { setTitle(s.title); setRenaming(true); }}>
    {isPinned && <span className="mr-1 text-teal-ink" title="Pinned">★</span>}{s.title}
  </button>;
  // Run settings, paid usage and cost limit show only when they differ from the defaults.
  const stateChips = <>
    <RunSettingsChip session={s} open={() => setView("summary")} />
    <PaidUsageToggle session={s} badge />
    <CostLimit session={s} badge />
  </>;
  const viewTabs = <Tabs tour="session.views" className="session-views mr-1 max-w-full border-b-0" label="Session view" value={summary ? "summary" : changesOpen ? "changes" : processesOpen ? "processes" : childrenOpen ? "subagents" : "trace"} onChange={(v) => (v === "changes" ? showChanges(onlyMain) : setView(v))} items={[
    { id: "summary", label: "Summary" },
    { id: "trace", label: "Trace" },
    ...(hasProcesses ? [{ id: "processes" as const, label: `Processes · ${runningProcesses.length}` }] : []),
    ...(children.length > 0 ? [{ id: "subagents" as const, icon: "agents" as const, label: `Subagents · ${children.filter((a) => a.status === "running").length || children.length}${children.some((a) => a.status === "running") ? " running" : ""}` }] : []),
    ...(isGit ? [{ id: "changes" as const, title: "Git changes of the session folder", label: `Changes${changed ? ` · ${git.data?.files.length || committedFiles || mainChanges}` : ""}` }] : []),
  ]} />;
  const hasFinal = rows.some((r) => r.t === "assistant");
  const actionsMenu = <Popover className="flex" viewport={phone} label={phone ? "Session actions" : undefined} align="right" width={phone ? 280 : 260} trigger={({ open, toggle }) => <button className={phone ? "iconbtn" : "iconbtn border border-line"} aria-label="Session actions" title="Session actions" aria-haspopup={phone ? "dialog" : undefined} aria-expanded={open} onClick={toggle}><Icon name="more" size={phone ? 20 : 14} /></button>}>
    {(close) => <>
      {!phone && !running && s.status !== "in_terminal" && <MenuItem icon={<Icon name="agents" size={14} />} label="Launch here" detail="A separate agent in the same folder, with this trace as context" onClick={() => { close(); useComposer.getState().followUpFrom(s); }} />}
      {hasFinal && <MenuItem icon={<Icon name="comment" size={14} />} label="Jump to the final message" detail="In the trace" onClick={() => { close(); setView("trace"); setJumpFinal((n) => n + 1); }} />}
      {isDone(s) && <MenuItem icon={<Icon name="mail" size={16} />} label="Mark as unread" detail="Return to Today for later" onClick={() => { close(); onClear(); void act(s.id, "session.unread"); }} />}
      {phone && <>
        {s.live && s.status === "waiting" && s.open_question && <ActionMenuItem name="session.stop" params={{ id: s.id }} close={close} detail="Interrupt the agent while it waits for your decision" />}
        <MenuItem icon={<Icon name="pencil" size={14} />} label="Rename" onClick={() => { close(); setTitle(s.title); setRenaming(true); }} />
        <MenuItem label="Session details" detail="Branch, usage and Launch here" onClick={() => { close(); setDetailsOpen(true); }} />
        <MenuItem icon={<Icon name="tag" size={14} />} label="Topics…" detail="Group related sessions across folders" onClick={() => { close(); setTopicsOpen(true); }} />
        {terminalsHere && <MenuItem icon={<Icon name="term" size={16} />} label="Terminal" detail={terminalBlock ?? "Open a shell in this folder (the checkout when its worktree is gone)"} disabled={!!terminalBlock} onClick={() => { close(); void openTerminal.folder(s.cwd, s.runner_id); }} />}
      </>}
      {s.status === "in_terminal" && tui && <MenuItem icon={<Icon name="term" size={14} />} label="Show TUI tab" detail="In the terminal dock" onClick={() => { close(); setDock({ open: true, active: tui.id }); }} />}
      {terminalsHere && !running && s.status !== "in_terminal" && s.status !== "quota_paused" && harness?.capabilities.terminal && <MenuItem icon={<Icon name="term" size={14} />} label={`Continue in ${name} TUI`} detail={terminalBlock ?? "In the terminal dock on the agent's runner, outside quota protection; Re-attach brings it back"} disabled={!!terminalBlock} onClick={() => { close(); void openTerminal.tui(s.id); }} />}
      {s.project_id && <MenuItem icon={<Icon name="folder" size={14} />} label="Browse files" detail="Read the folder in Files" onClick={() => { close(); void navigate({ to: "/projects/$id/code", params: { id: s.project_id! }, search: { root: s.cwd, session: s.id } }); }} />}
      <MenuItem icon={<Icon name="agents" size={14} />} label="Use trace as context" detail="Reference this trace in a new agent's prompt" disabled={s.last_seq === 0} onClick={() => { close(); useComposer.getState().start({ agentTraces: [{ session_id: s.id, through_seq: s.last_seq }] }); }} />
      {s.status !== "closed" && !archivePrimary && <MenuItem icon={<Icon name="archive" size={14} />} label="Archive" detail="It stays resumable" onClick={() => { close(); archive(); }} />}
      <MenuItem icon={<span className="text-teal-ink">{isPinned ? "★" : "☆"}</span>} label={isPinned ? "Unpin session" : "Pin session"} onClick={() => { close(); togglePin(s.id); }} />
    </>}
  </Popover>;

  return (
    <div className={`relative flex h-full min-h-0 min-w-0 flex-col bg-paper ${readingExpanded ? "session-expanded" : ""}`}
      onDragOver={(e) => { if (e.dataTransfer.types.includes("Files")) { e.preventDefault(); setDragging(true); } }}
      onDragLeave={(e) => { if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setDragging(false); }}
      onDrop={(e) => { if (e.dataTransfer.types.includes("Files")) { e.preventDefault(); setDragging(false); dropImages.current?.(Array.from(e.dataTransfer.files)); } }}
    >
      {dragging && <div className="pointer-events-none absolute inset-0 z-10 grid place-items-center border-2 border-teal bg-teal-soft/80 font-semibold text-teal-ink">{harness?.capabilities.images ? "Drop files or images to attach to your message" : `Drop files to attach (${name} does not accept images)`}</div>}
      {readingExpanded && <div className="flex min-h-11 shrink-0 items-center gap-2 border-b border-line-2 px-3 py-1">
        <span className="min-w-0 flex-1 truncate text-[13px] font-semibold" title={s.title}>{s.title}</span>
        <SessionStatus s={s} />
        {!archivePrimary && <StatePrimary s={s} primary={primary} />}
      </div>}
      {phone ? <><PhoneBar title={<>{isPinned && <span className="mr-1 text-teal-ink" title="Pinned">★</span>}{s.title}</>} onBack={onClear} backLabel="Back to agents" end={actionsMenu} />
      <div className="session-header @container grid items-center border-b border-line-2">
        {renaming && <div className="session-heading flex min-w-0 items-center gap-1">{titleEl}</div>}
        <span className="session-meta flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-[12px] text-ink-3 tabular-nums">
          <SessionStatus s={s} />
          {s.launch_spec.bot && <Link to="/bots/$id" params={{ id: s.launch_spec.bot.id }} search={(s.launch_spec.bot.thread && s.launch_spec.bot.thread !== "main" ? { thread: s.launch_spec.bot.thread } : {}) as never} className="flex min-w-0 max-w-[40%] shrink-0 no-underline" aria-label={`Open the chat of ${s.launch_spec.bot.name}`}><BotChip name={s.launch_spec.bot.name} className="max-w-full" /></Link>}
          {localPr && <LocalPrChip pr={localPr} />}
          <SessionTopics sessionId={s.id} open={topicsOpen} onClose={() => setTopicsOpen(false)} />
          <button className="session-details min-w-0 flex-1 truncate text-left" aria-label="Session details" onClick={() => setDetailsOpen(true)}>{place} <Icon name="chev" size={12} /></button>
          {stateChips}
        </span>
        <div className="session-toolbar order-4 col-span-2 flex flex-wrap items-center gap-1 pt-1">
          {viewTabs}
          <div className="session-actions ml-auto flex flex-wrap items-center gap-1">
            {canExpand && <button className="iconbtn session-expand" aria-label={readingExpanded ? "Collapse session view" : "Expand session view"} title={readingExpanded ? "Collapse session view" : "Expand session view"} aria-pressed={readingExpanded} onClick={() => setExpanded((value) => !value)}><Icon name={readingExpanded ? "collapse" : "expand"} size={14} /></button>}
          </div>
        </div>
      </div></> : <div className="session-header @container grid gap-0.5 border-b border-line-2 pt-1.5 pr-2.5 pl-3">
        {/* Row 1: title, the one status, the state's primary action, Terminal and ⋯. Row 2: views and one muted meta line. */}
        <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
          <div className="session-heading flex min-w-0 flex-1 basis-56 items-center gap-2">
            <HarnessMark harness={s.harness} size={16} />
            {s.launch_spec.bot && <Link to="/bots/$id" params={{ id: s.launch_spec.bot.id }} search={(s.launch_spec.bot.thread && s.launch_spec.bot.thread !== "main" ? { thread: s.launch_spec.bot.thread } : {}) as never} className="flex min-w-0 max-w-[40%] shrink-0 no-underline hover:opacity-80" aria-label={`Open ${s.launch_spec.bot.name}`}><BotChip name={s.launch_spec.bot.name} className="max-w-full" /></Link>}
            {titleEl}
            {!renaming && <SessionStatus s={s} />}
          </div>
          <div className="session-actions ml-auto flex flex-wrap items-center gap-1">
            <span data-tour="session.primary" className="inline-flex items-center gap-1"><StatePrimary s={s} primary={primary} /></span>
            <TerminalMenu cwd={s.cwd} runner={s.runner_id} small label={<span className="@max-[520px]:sr-only">Terminal</span>} title={`Open a shell in ${s.cwd}`} />
            {actionsMenu}
            <button className="iconbtn shrink-0" title="Close panel" aria-label="Close panel" onClick={onClear}><Icon name="x" size={14} /></button>
          </div>
        </div>
        <div className="flex min-w-0 flex-wrap items-center gap-x-3">
          {viewTabs}
          <span className="flex shrink-0 items-center gap-1.5 text-[12px] whitespace-nowrap empty:hidden">{stateChips}</span>
          {/* The meta line and ⓘ wrap together under the views when less than 192 px remain beside them. */}
          <div className="flex min-w-0 flex-1 basis-48 items-center gap-1">
          <span className="session-meta flex min-w-0 flex-1 items-center gap-x-1.5 overflow-hidden text-[12px] whitespace-nowrap text-ink-3 tabular-nums">
            <span className="max-w-[40%] shrink-0 truncate" title={s.cwd}>{place}</span>
            <span className="flex min-w-0 [&>div]:min-w-0 [&_button]:max-w-full"><WorktreeChip session={s} /></span>
            <SessionTopics sessionId={s.id} />
            {localPr && <LocalPrChip pr={localPr} />}
            {(profileLabel || contextShare) && <span className="min-w-0 truncate">{[profileLabel, contextShare].filter(Boolean).join(" · ")}</span>}
            </span>
          {/* Outside the clipped meta line (and portaled), so ⓘ and its panel stay visible when the line is cut. */}
          <Popover viewport align="right" width={280} className="shrink-0" trigger={({ toggle }) => <button className="iconbtn shrink-0 text-ink-3" title="Session details" aria-label="Session details" onClick={toggle}><Icon name="info" size={14} /></button>}>
            {() => <dl className="grid gap-2 p-2 text-[12px] text-ink-2">
              <div><dt className="text-[11px] text-ink-3">Folder</dt><dd className="break-all">{s.cwd}</dd></div>
              {s.launch_spec.profile && <div><dt className="text-[11px] text-ink-3">Profile</dt><dd>{name} · {profileName ?? `${s.launch_spec.profile} (deleted profile)`}</dd></div>}
              {s.model && <div><dt className="text-[11px] text-ink-3">Model</dt><dd>{s.model}{s.effort ? ` · ${s.effort}` : ""}</dd></div>}
              {usageLine(s) && <div><dt className="text-[11px] text-ink-3">Usage</dt><dd>{usageLine(s)}</dd></div>}
              <div><dt className="text-[11px] text-ink-3">Started</dt><dd>{when(s.started_at)}</dd></div>
            </dl>}
          </Popover>
          </div>
        </div>
      </div>}
      {phone && detailsOpen && <Sheet title="Session details" onClose={() => setDetailsOpen(false)}>
        <div className="session-detail-sheet grid gap-4 p-4">
          <div className="flex items-start gap-2"><HarnessMark harness={s.harness} size={20} /><div className="min-w-0"><b className="break-words">{s.title}</b><p className="mt-1 text-ink-3">{projectName(projects, s.project_id, s.cwd)} · {s.launch_spec.profile ? profileName ?? `${s.launch_spec.profile} (deleted profile)` : name}</p></div></div>
          <dl className="grid gap-2 text-ink-2">
            {usageLine(s) && <div><dt className="text-[12px] text-ink-3">Usage</dt><dd>{usageLine(s)}</dd></div>}
            <div><dt className="text-[12px] text-ink-3">Started</dt><dd>{when(s.started_at)}</dd></div>
          </dl>
          {localPr && !prInCard && <div className="flex flex-wrap gap-2"><LocalPrButtons pr={localPr} /></div>}
          <SessionWorktreeDetails session={s} />
          <FollowUpAgent session={s} onPrepared={() => setDetailsOpen(false)} />
        </div>
      </Sheet>}

      {branch.stopped && !running && <div className="grid gap-1 border-b border-attn-line bg-attn-soft px-4 py-2.5">
        <NotMerged branch={branch} />
        <span className="text-[12px] text-attn">{s.status === "waiting" ? "Fix merge sends the problem to the agent: it rebases, resolves and asks to merge again." : "Resume the agent, then use Fix merge."}</span>
      </div>}
      {log?.error && <div role="alert" className="flex flex-wrap items-center gap-3 px-4 py-3 text-ink-3">
        <span className="min-w-0 flex-1 break-words">Could not load session events: {log.error}</span>
        <button className="btn btn-sm" disabled={log.loading} onClick={() => void load(sessionId)}>{log.loading ? "Loading…" : "Retry"}</button>
      </div>}
      {s.capacity_retry_at != null && <div role="status" className="flex flex-wrap items-center gap-2 border-b border-line bg-teal-soft px-3 py-2 text-[12px] text-teal-ink">
        <Icon name="clock" size={16} />Model at capacity · automatic retry {s.capacity_retry_count + 1} scheduled for {when(s.capacity_retry_at)}. Resume now or archive to cancel.
      </div>}
      {pollProcesses && processesError != null && <div role="alert" className="px-3 py-2 text-[12px] text-danger">{String(processesError)} <button className="btn btn-sm" onClick={() => void processes.refetch()}>Retry</button></div>}
      {hasProcesses && !processesOpen && !running && <div className="flex items-center gap-2 px-3 py-2 text-[12px] text-ink-3">Background commands are still running.<button className="btn btn-sm" onClick={() => setView("processes")}>View processes</button></div>}
      {processesOpen && <ProcessesPanel sessionId={s.id} processes={runningProcesses} refresh={() => processes.refetch()} />}
      {changesOpen && <GitPanel key={`${s.id}-${gitMain}`} initialMain={gitMain} sessionId={s.id} canMessage={s.status !== "in_terminal" && !quotaMessageBlocked && !(running && !harness?.capabilities.messages_while_running)} />}
      <CodeLinksContext.Provider value={codeLinks}>
      <ShellCodeContext.Provider value={terminalBlock ? null : s.cwd}>
      {childrenOpen && log && <SubagentsPanel key={s.id} log={log} agents={children} sessionId={s.id} live={s.live} back={() => setView("trace")} />}
      {!changesOpen && !childrenOpen && !processesOpen && summary && (log && !log.loaded ? <div className="flex-1 p-6 text-center text-ink-3">{log.error ? "Session events unavailable." : "Loading…"}</div> : <SessionSummary key={s.id} session={s} rows={rows} earlier={log?.earlier ?? null} localPr={localPr} reviewDiff={showChanges} showTrace={() => setView("trace")} />)}
      {traceOpen && !running && <><StateBand session={s} /><FollowUpBands sessionId={s.id} /><QuotaBand session={s} /></>}
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
          <div className="flex items-center gap-2 pb-2 text-[12px] text-work">
            <span className="lp-spin" />
            {s.status === "starting" ? "Starting…" : s.status === "background" ? "Background work is still running…" : "Working…"}
          </div>
        )}
      </div>
      </ShellCodeContext.Provider>
      </CodeLinksContext.Provider>
      <div className={readingExpanded ? "hidden" : "contents"}>
      {canReply && (!phone || replying) && <MessageBox onHide={phone ? () => setReplying(false) : undefined} acceptsImages={!!harness?.capabilities.images} onSent={() => setView("trace")} key={s.id} dropImages={dropImages} inputRef={inputRef} sessionId={s.id} placeholder={!quotaPaused && s.open_question && ask?.permission ? "Choose a decision on the approval card." : placeholder} disabled={!!(!quotaPaused && s.open_question && ask?.permission) || s.status === "in_terminal" || quotaMessageBlocked || (running && !s.open_question && !harness?.capabilities.messages_while_running)} ask={!quotaPaused && s.open_question && s.live ? ask : null} />}
      {/* Phone: the main action and Reply under your thumb, while the message box is closed. */}
      {phone && !(canReply && replying) && (canReply || primary) && <div className="session-bottom-bar flex flex-none items-center gap-2 border-t border-line-2 px-3 py-2" data-tour="session.bottom-bar">
        {canReply && <button className="btn flex-1" onClick={() => { setReplying(true); requestAnimationFrame(() => inputRef.current?.focus()); }}>
          <Icon name="comment" size={14} />Reply
        </button>}
        {primary && <span className="session-bottom-primary flex min-w-0 flex-1 items-center gap-2"><StatePrimary s={s} primary={primary} /></span>}
      </div>}
      </div>
    </div>
  );
}
