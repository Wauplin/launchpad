// One session in a pane: header with state and actions, the conversation, and the message box.
// The conversation is virtualized (spike 4 rules: memoized rows, no flushSync).

import { FollowUpAgent } from "./FollowUpAgent";
import { HostLabel } from "@/components/HostLabel";
import { hostOf, linkLabel, useHost } from "@/lib/hosts";
import { useProfile } from "@/lib/profiles";
import { SessionSummary, StateBand, QuotaBand, FollowUpBands, CostLimit, PaidUsageToggle, PermissionSwitch } from "./SessionSummary";
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
import type { ImageAttachment } from "@/gen/ImageAttachment";
import { HarnessMark, useHarnessInfo } from "@/components/HarnessMark";
import { CodeLinksContext, ShellCodeContext, type CodeLinks } from "@/components/Markdown";
import { pathResolver, useCodeFiles } from "@/features/code/lib";
import { useMessageDrafts } from "@/stores/messages";
import { isDone, isRunning } from "@/lib/sessionStatus";
import { Icon } from "@/components/Icon";
import { toastError } from "@/components/Toast";
import { abandonOperation } from "@/lib/operations";
import { api, ApiError } from "@/lib/api";
import { attachmentSrc, pastedImages, readAttachments } from "@/lib/attachments";
import { projectName, useProjects, useSession } from "@/lib/queries";
import { emptyLog, useSessionLogs } from "@/lib/sessionStore";
import { buildRows } from "./rows";
import { EventRow } from "./EventRow";
import { foldTrace } from "./traceRows";
import { MenuItem, Popover } from "@/components/Popover";
import type { SessionGitStatus } from "@/gen/SessionGitStatus";
import { StatusBadge, usageLine } from "./SessionBits";
import { useOpenTerminal, useTerminals } from "@/features/terminals/lib";
import { Sheet } from "@/mobile/Sheet";
import { WorktreeCard, useSessionWorktrees, WorktreeChip } from "./Worktrees";
import { usePhone } from "@/lib/phone";

function act(id: string, action: string) {
  return api.post(`/api/sessions/${id}/${action}`).catch(toastError);
}

type PendingAsk = { request_id: string; count: number; permission: boolean } | null;

function MessageBox({ acceptsImages, sessionId, placeholder, disabled, ask, dropImages, inputRef: ref, onSent }: { acceptsImages: boolean; sessionId: string; placeholder: string; disabled: boolean; ask: PendingAsk; dropImages: React.RefObject<((files: File[]) => void) | null>; inputRef: React.RefObject<HTMLTextAreaElement | null>; onSent: () => void }) {
  const text = useMessageDrafts((s) => s.drafts[sessionId] ?? "");
  const setDraft = useMessageDrafts((s) => s.set);
  const setText = (t: string) => setDraft(sessionId, t);
  const [busy, setBusy] = useState(false);
  const [delivery, setDelivery] = useState<"sending" | "delivered" | "uncertain" | "failed" | null>(null);
  const [images, setImages] = useState<ImageAttachment[]>([]);
  const [reading, setReading] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);
  const readingRef = useRef(false);
  const addImages = async (input: File[] | Promise<File[]>) => {
    if (!acceptsImages || disabled || busy || readingRef.current) return;
    readingRef.current = true;
    setReading(true);
    try {
      const files = await input;
      if (!files.length) return;
      const added = await readAttachments(files, images, true);
      setImages((current) => [...current, ...added]);
    } catch (e) { toastError(e); }
    finally { readingRef.current = false; setReading(false); }
  };
  useLayoutEffect(() => { dropImages.current = (files) => { void addImages(files); }; });
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 160)}px`;
  }, [text, ref]);
  const send = async () => {
    const t = text.trim();
    if ((!t && !images.length) || busy || disabled || reading || ask?.permission) return;
    setBusy(true);
    setDelivery("sending");
    try {
      if (ask) {
        // A typed answer to the open question.
        await api.post(`/api/sessions/${sessionId}/answer`, { request_id: ask.request_id, answers: Array(ask.count).fill(t || "See the attached images in my next message.") });
        if (images.length) await api.post(`/api/sessions/${sessionId}/input`, { text: t, images });
      } else {
        await api.post(`/api/sessions/${sessionId}/input`, { text: t, images });
      }
      setDelivery("delivered");
      onSent();
      setText("");
      setImages([]);
    } catch (e) {
      setDelivery(e instanceof ApiError && e.operationCompleted ? "failed" : "uncertain");
      toastError(e);
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="border-t border-line-2 px-2.5 pt-2 pb-2.5">
      <input ref={fileRef} type="file" accept="image/*" multiple hidden onChange={(e) => { void addImages(Array.from(e.target.files ?? [])); e.target.value = ""; }} />
      {images.length > 0 && <div className="mb-2 flex flex-wrap gap-2">{images.map((image, i) => <div key={i} className="relative">
        <img src={attachmentSrc(image.data_url)} alt={image.name} title={image.name} className="h-16 max-w-28 rounded-md border border-line object-contain" />
        <button className="iconbtn absolute -top-1 -right-1 bg-paper!" title={`Remove ${image.name}`} disabled={busy || reading} onClick={() => setImages((current) => current.filter((_, j) => j !== i))}><Icon name="x" size={12} /></button>
      </div>)}</div>}
      {delivery && <div role="status" className="mb-1 text-[11.5px] text-ink-3">{delivery === "sending" ? "◷ Sending…" : delivery === "delivered" ? "✓ Delivered" : delivery === "failed" ? "Attempt failed. Check the trace before starting a new attempt." : "Delivery not confirmed. Retry the same message to check its result."}</div>}
      {delivery === "uncertain" && <button className="btn btn-sm mb-1" disabled={busy} onClick={() => void abandonOperation(`/api/sessions/${sessionId}/input`, { text: text.trim(), images }).then(() => setDelivery(null)).catch(toastError)}>I checked the trace — start a new attempt</button>}
      {reading && <div className="mb-1 text-[11.5px] text-ink-3">Reading images…</div>}
      <div className="session-message-input flex items-end gap-2 rounded-lg border border-line px-2.5 py-[7px] focus-within:border-teal focus-within:shadow-[0_0_0_3px_var(--color-teal-soft)]">
        <textarea
          ref={ref}
          rows={1}
          value={text}
          disabled={disabled || busy || reading}
          onPaste={(e) => {
            if (!acceptsImages || disabled || busy || readingRef.current) return;
            const files = pastedImages(e);
            if (files) void addImages(files);
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
          className="max-h-40 min-h-5 min-w-0 flex-1 resize-none bg-transparent text-[12.5px] leading-5 outline-none disabled:opacity-60"
        />
        {acceptsImages && <button className="iconbtn" title="Attach images (you can also drop or paste them)" aria-label="Attach images" disabled={disabled || busy || reading} onClick={() => fileRef.current?.click()}>
          <Icon name="clip" size={14} />
        </button>}
        <button className="iconbtn border-line! bg-paper" title={delivery === "failed" ? "Send as a new attempt" : delivery === "uncertain" ? "Retry delivery" : "Send"} disabled={disabled || busy || reading || (!text.trim() && !images.length)} onClick={() => void send()}>
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

export function SessionPane({ sessionId, onClear, initialView }: { sessionId: string; onClear: () => void; initialView?: "changes" }) {
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
  const [view, setView] = useState<View | null>(initialView ?? null);
  useEffect(() => { if (s && view === null) setView(isRunning(s) ? "trace" : "summary"); else if (s && isRunning(s) && view === "summary") setView("trace"); }, [s, view]);
  const git = useQuery({ queryKey: ["session-git", sessionId], queryFn: () => api.get<SessionGitStatus>(`/api/sessions/${sessionId}/git`), refetchInterval: 5000 });
  const isGit = !!git.data;
  const current: View = view === "processes" && !hasProcesses ? "trace" : view === "changes" && !isGit ? "summary" : view ?? "summary";
  const summary = !s || current === "summary" && !isRunning(s);
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
  const trace = useMemo(() => foldTrace(rows, typed), [rows, typed]);
  const ask: PendingAsk = useMemo(() => {
    for (let i = rows.length - 1; i >= 0; i--) {
      const r = rows[i];
      if (r.t === "ask" && !r.answers) return { request_id: r.request_id, count: r.questions.length, permission: r.questions.some((q) => q.permission) };
    }
    const earlier = log?.earlier?.last_ask?.body;
    return earlier?.kind === "ask_user" && !log?.earlier?.last_ask_answers ? { request_id: earlier.request_id, count: earlier.questions.length, permission: earlier.questions.some((q) => q.permission) } : null;
  }, [rows, log?.earlier]);

  const scrollRef = useRef<HTMLDivElement>(null);
  const atBottom = useRef(true);
  const traceOpen = !summary && !changesOpen && !childrenOpen && !processesOpen;
  const v = useVirtualizer({ enabled: traceOpen, count: trace.length, getScrollElement: () => scrollRef.current, estimateSize: () => 48, overscan: 6, useFlushSync: false });
  const total = v.getTotalSize();
  useLayoutEffect(() => {
    if (atBottom.current && trace.length) v.scrollToIndex(trace.length - 1, { align: "end" });
  }, [trace.length, total, v, summary, changesOpen]);
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
  const changed = uncommitted || mainChanges > 0;
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
      {dragging && <div className="pointer-events-none absolute inset-0 z-10 grid place-items-center border-2 border-teal bg-teal-soft/80 font-semibold text-teal-ink">{harness?.capabilities.images ? "Drop images to attach to your message" : `${name} does not accept images`}</div>}
      <div className="session-header grid grid-cols-[18px_1fr] items-center gap-x-[9px] gap-y-0.5 border-b border-line-2 py-2 pr-2.5 pl-3">
        <span className={`order-1 row-span-2 self-start pt-px ${phone ? "hidden" : ""}`}>
          <HarnessMark harness={s.harness} size={18} />
        </span>
        <div className="session-heading order-2 flex min-w-0 items-center gap-1">
          {renaming ? <div className="flex min-w-0 flex-1 flex-wrap items-center gap-1">
            <input autoFocus aria-label="Session title" className="field-input min-w-0 flex-1 basis-32 text-[12.5px] font-[650]" maxLength={100} value={title} disabled={savingTitle} onChange={(e) => setTitle(e.target.value)} />
            <button className="btn btn-sm" disabled={!title.trim() || savingTitle} onClick={async () => { setSavingTitle(true); try { await api.post(`/api/sessions/${s.id}/rename`, { title: title.trim() }); setRenaming(false); } catch (e) { toastError(e); } finally { setSavingTitle(false); } }}>Save title</button>
            <button className="btn btn-sm" disabled={savingTitle} onClick={() => setRenaming(false)}>Cancel</button>
          </div> : <button className="session-title min-w-0 flex-1 truncate text-left text-[12.5px] font-[650] hover:text-teal-ink" title={`${s.title} · Click to edit`} aria-label={`Edit session title: ${s.title}`} onClick={() => { setTitle(s.title); setRenaming(true); }}>
            {isPinned && <span className="mr-1 text-teal-ink" title="Pinned">★</span>}{s.title}
          </button>}
          {phone
            ? <button className="iconbtn session-back order-first shrink-0 self-start" title="Back to agents" aria-label="Back to agents" onClick={onClear}><Icon name="chev" size={20} className="rotate-90" /></button>
            : <button className="iconbtn shrink-0 self-start" title="Close panel" aria-label="Close panel" onClick={onClear}>
              <Icon name="x" size={14} />
            </button>}
        </div>
        <span className="session-meta order-3 col-start-2 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-[11.5px] text-ink-3 tabular-nums">
          {phone ? <>
            <HostLabel path={s.cwd} />
            <StatusBadge s={s} />
            <button className="session-details min-w-0 flex-1 truncate text-left" aria-label="Session details" onClick={() => setDetailsOpen(true)}>{s.launch_spec.target.kind === "browser" ? "Browser · read only" : projectName(projects, s.project_id, s.cwd)} <Icon name="chev" size={12} /></button>
            <span className="ml-auto flex flex-wrap justify-end gap-1">{primary}</span>
            <PermissionSwitch session={s} />
            <PaidUsageToggle session={s} badge />
            <CostLimit session={s} badge />
          </> : <>
            {localPr && <button className="text-teal-ink hover:underline" onClick={() => void navigate({ to: "/predrafts/$id", params: { id: localPr.id } })}>Pre-draft PR</button>}
            <span className="truncate">{s.launch_spec.target.kind === "browser" ? "Browser · read only" : projectName(projects, s.project_id, s.cwd)}</span>
            <WorktreeChip session={s} />
            {s.launch_spec.profile && <span title="Harness profile">{profileName ?? `${s.launch_spec.profile} (deleted profile)`}</span>}
            <HostLabel path={s.cwd} />
            <StatusBadge s={s} />
            <PermissionSwitch session={s} />
            <PaidUsageToggle session={s} badge />
            <CostLimit session={s} badge />
            <span>{usageLine(s)}</span>
            <span className="ml-auto" title="Started">{new Date(s.started_at).toLocaleDateString()}</span>
          </>}
        </span>
        <div className="session-toolbar order-4 col-span-2 flex flex-wrap items-center gap-1 pt-1">
          <div className="session-views mr-1 inline-flex max-w-full flex-wrap gap-0.5 rounded border border-line p-0.5" role="group" aria-label="Session view">
            <button className={`btn btn-sm ${summary && !phone ? "bg-select!" : ""}`} aria-pressed={summary} disabled={running} onClick={() => setView("summary")}>Summary</button>
            <button className={`btn btn-sm ${traceOpen && !phone ? "bg-select!" : ""}`} aria-pressed={traceOpen} onClick={() => setView("trace")}>Trace</button>
            {hasProcesses && <button className={`btn btn-sm ${processesOpen && !phone ? "bg-select!" : ""}`} aria-pressed={processesOpen} onClick={() => setView("processes")}>Processes · {runningProcesses.length}</button>}
            {children.length > 0 && <button className={`btn btn-sm ${childrenOpen && !phone ? "bg-select!" : ""}`} aria-pressed={childrenOpen} onClick={() => setView("subagents")}><Icon name="agents" size={12} />Subagents · {children.filter((a) => a.status === "running").length || children.length}{children.some((a) => a.status === "running") ? " running" : ""}</button>}
            {isGit && <button className={`btn btn-sm ${changesOpen && !phone ? "bg-select!" : ""}`} aria-pressed={changesOpen} title="Git changes of the session folder" onClick={() => showChanges(onlyMain)}>Changes{changed ? ` · ${git.data!.files.length || mainChanges}` : ""}</button>}
          </div>
          <div className="session-actions ml-auto flex flex-wrap items-center gap-1">
            {!phone && <>
              {primary}
              <FollowUpAgent session={s} />
              {s.launch_spec.target.kind !== "browser" && <button className="btn btn-sm" title={`Open a shell in ${s.cwd}`} onClick={() => void openTerminal.shell(s.cwd)}>
                <Icon name="term" size={12} />
                Terminal
              </button>}
            </>}
            <Popover viewport={phone} label={phone ? "Session actions" : undefined} align="right" width={phone ? 280 : 260} trigger={({ open, toggle }) => <button className={`${phone ? "iconbtn" : "btn btn-sm"} ${open ? "bg-select!" : ""}`} aria-label="Session actions" title="Session actions" aria-haspopup={phone ? "dialog" : undefined} aria-expanded={open} onClick={toggle}>{phone ? <span aria-hidden="true" className="text-xl leading-none">⋯</span> : <>More<Icon name="chev" size={12} /></>}</button>}>
              {(close) => <>
                {isDone(s) && <MenuItem icon={<Icon name="mail" size={16} />} label="Mark as unread" detail="Return to Needs you for later" onClick={() => { close(); onClear(); void act(s.id, "unread"); }} />}
                {phone && <>
                  <MenuItem label="Session details" detail="Branch, usage and follow up" onClick={() => { close(); setDetailsOpen(true); }} />
                  {s.launch_spec.target.kind !== "browser" && <MenuItem icon={<Icon name="term" size={16} />} label="Terminal" detail="Open a shell in this folder" onClick={() => { close(); void openTerminal.shell(s.cwd); }} />}
                </>}
                {s.status === "in_terminal" && tui && <MenuItem label="Show TUI" detail="In the terminal dock" onClick={() => { close(); setDock({ open: true, active: tui.id }); }} />}
                {s.launch_spec.target.kind !== "browser" && !running && s.status !== "in_terminal" && s.status !== "quota_paused" && harness?.capabilities.terminal && <MenuItem icon={<Icon name="term" size={14} />} label="Open in terminal" detail={`Continue in the ${name} TUI, outside quota protection`} onClick={() => { close(); void openTerminal.tui(s.id); }} />}
                {s.project_id && <MenuItem icon={<Icon name="file" size={14} />} label="Files" detail="Read the folder in the code viewer" onClick={() => { close(); void navigate({ to: "/projects/$id/code", params: { id: s.project_id! }, search: { root: s.cwd, session: s.id } }); }} />}
                <MenuItem icon={<Icon name="agents" size={14} />} label="Use trace as context" detail="Reference this trace in a new agent's prompt" disabled={s.last_seq === 0} onClick={() => { close(); useComposer.getState().start({ agentTraces: [{ session_id: s.id, through_seq: s.last_seq }] }); }} />
                {s.status !== "closed" && !archivePrimary && <MenuItem icon={<Icon name="archive" size={14} />} label="Archive" detail="It stays resumable" onClick={() => { close(); archive(); }} />}
                <MenuItem icon={<span className="text-teal-ink">{isPinned ? "★" : "☆"}</span>} label={isPinned ? "Unpin session" : "Pin session"} onClick={() => { close(); togglePin(s.id); }} />
              </>}
            </Popover>
          </div>
        </div>
      </div>
      {phone && detailsOpen && <Sheet title="Session details" onClose={() => setDetailsOpen(false)}>
        <div className="session-detail-sheet grid gap-4 p-4 text-sm">
          <div className="flex items-start gap-2"><HarnessMark harness={s.harness} size={20} /><div className="min-w-0"><b className="break-words">{s.title}</b><p className="mt-1 text-ink-3">{s.launch_spec.target.kind === "browser" ? "Browser · read only" : projectName(projects, s.project_id, s.cwd)} · {s.launch_spec.profile ? profileName ?? `${s.launch_spec.profile} (deleted profile)` : name}</p></div></div>
          <dl className="grid gap-2 text-ink-2">
            {usageLine(s) && <div><dt className="text-xs text-ink-3">Usage</dt><dd>{usageLine(s)}</dd></div>}
            <div><dt className="text-xs text-ink-3">Started</dt><dd>{new Date(s.started_at).toLocaleString()}</dd></div>
          </dl>
          {localPr && <button className="btn" onClick={() => { setDetailsOpen(false); void navigate({ to: "/predrafts/$id", params: { id: localPr.id } }); }}>Pre-draft PR</button>}
          <SessionWorktreeDetails session={s} />
          <FollowUpAgent session={s} onPrepared={() => setDetailsOpen(false)} />
        </div>
      </Sheet>}

      {log?.error && <div role="alert" className="flex flex-wrap items-center gap-3 px-4 py-3 text-ink-3">
        <span className="min-w-0 flex-1 break-words">Could not load session events: {log.error}</span>
        <button className="btn btn-sm" disabled={log.loading} onClick={() => void load(sessionId)}>{log.loading ? "Loading…" : "Retry"}</button>
      </div>}
      <HostBand cwd={s.cwd} running={running} />
      {s.launch_spec.target.kind === "browser" && <div className="border-b border-line bg-teal-soft px-3 py-1 text-xs text-teal-ink">Browser · read only</div>}
      {s.live && processes.isError && <div role="alert" className="px-3 py-2 text-xs text-danger">{String(processes.error)} <button className="btn btn-sm" onClick={() => void processes.refetch()}>Retry</button></div>}
      {hasProcesses && !processesOpen && !running && <div className="flex items-center gap-2 px-3 py-2 text-xs text-ink-3">Background commands are still running.<button className="btn btn-sm" onClick={() => setView("processes")}>View processes</button></div>}
      {processesOpen && <ProcessesPanel sessionId={s.id} processes={runningProcesses} refresh={() => processes.refetch()} />}
      {changesOpen && <GitPanel key={`${s.id}-${gitMain}`} initialMain={gitMain} sessionId={s.id} canMessage={s.status !== "in_terminal" && s.status !== "quota_paused" && !(running && !harness?.capabilities.messages_while_running)} />}
      <CodeLinksContext.Provider value={codeLinks}>
      <ShellCodeContext.Provider value={s.launch_spec.target.kind === "browser" ? null : s.cwd}>
      {childrenOpen && log && <SubagentsPanel key={s.id} log={log} agents={children} sessionId={s.id} live={s.live} back={() => setView("trace")} />}
      {!changesOpen && !childrenOpen && !processesOpen && summary && (log && !log.loaded ? <div className="flex-1 p-6 text-center text-ink-3">{log.error ? "Session events unavailable." : "Loading…"}</div> : <SessionSummary key={s.id} session={s} rows={rows} earlier={log?.earlier ?? null} localPr={localPr} reviewDiff={showChanges} showTrace={() => setView("trace")} />)}
      {traceOpen && !running && <><StateBand session={s}>
        <button className="btn btn-sm" disabled={!rows.some((r) => r.t === "assistant")} onClick={() => { atBottom.current = false; const i = trace.findLastIndex((r) => r.t === "assistant"); if (i >= 0) v.scrollToIndex(i, { align: "start" }); }}>Final message</button>
        <button className="btn btn-sm" onClick={() => setView("summary")}>Summary</button>
      </StateBand><FollowUpBands sessionId={s.id} /><QuotaBand session={s} /></>}
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
                    const added = foldTrace(buildRows(useSessionLogs.getState().logs[s.id]), typed).length - before;
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
                key={trace[it.index].key}
                data-index={it.index}
                ref={v.measureElement}
                className="flex flex-col py-[5px]"
                style={{ position: "absolute", top: 0, left: 0, right: 0, transform: `translateY(${it.start}px)` }}
              >
                <EventRow row={trace[it.index]} sessionId={s.id} live={s.live} />
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
