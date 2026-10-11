// One session in a pane: header with state and actions, the conversation, and the message box.
// The conversation is virtualized (spike 4 rules: memoized rows, no flushSync).

import { ResumeOtherAgent } from "./ResumeOtherAgent";
import { useVariant } from "@/lib/variants";
import { SessionSummary, StateBand, QuotaBand, FollowUpBands, CostLimit, PaidUsageToggle } from "./SessionSummary";
import { SubagentsPanel, subagents } from "./SubagentsPanel";
import { GitPanel } from "./GitPanel";
import { useComposer } from "@/stores/composer";
import { useLayout } from "@/stores/layout";
import { useQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { ImageAttachment } from "@/gen/ImageAttachment";
import { HarnessMark, useHarnessInfo } from "@/components/HarnessMark";
import { CodeLinksContext, type CodeLinks } from "@/components/Markdown";
import { pathResolver, useCodeFiles } from "@/features/code/lib";
import { useMessageDrafts } from "@/stores/messages";
import { isRunning } from "@/lib/sessionStatus";
import { Icon } from "@/components/Icon";
import { toastError } from "@/components/Toast";
import { api } from "@/lib/api";
import { clipboardImages, readAttachments } from "@/lib/attachments";
import { projectName, useProjects, useSession } from "@/lib/queries";
import { useSessionLogs } from "@/lib/sessionStore";
import { buildRows } from "./rows";
import { EventRow } from "./EventRow";
import { foldTrace } from "./traceRows";
import { MenuItem, Popover } from "@/components/Popover";
import type { SessionGitStatus } from "@/gen/SessionGitStatus";
import { StatusBadge, usageLine } from "./SessionBits";
import { useOpenTerminal, useTerminals } from "@/features/terminals/lib";
import { WorktreeChip } from "./Worktrees";

function act(id: string, action: string) {
  return api.post(`/api/sessions/${id}/${action}`).catch(toastError);
}

type PendingAsk = { request_id: string; count: number } | null;

function MessageBox({ acceptsImages, sessionId, placeholder, disabled, ask, dropImages, inputRef: ref, onSent }: { acceptsImages: boolean; sessionId: string; placeholder: string; disabled: boolean; ask: PendingAsk; dropImages: React.RefObject<((files: File[]) => void) | null>; inputRef: React.RefObject<HTMLTextAreaElement | null>; onSent: () => void }) {
  const text = useMessageDrafts((s) => s.drafts[sessionId] ?? "");
  const setDraft = useMessageDrafts((s) => s.set);
  const setText = (t: string) => setDraft(sessionId, t);
  const [busy, setBusy] = useState(false);
  const [images, setImages] = useState<ImageAttachment[]>([]);
  const [reading, setReading] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);
  const readingRef = useRef(false);
  const addImages = async (files: File[]) => {
    if (!acceptsImages || disabled || busy || readingRef.current || !files.length) return;
    readingRef.current = true;
    setReading(true);
    try {
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
    if ((!t && !images.length) || busy || disabled || reading) return;
    setBusy(true);
    try {
      if (ask) {
        // A typed answer to the open question.
        await api.post(`/api/sessions/${sessionId}/answer`, { request_id: ask.request_id, answers: Array(ask.count).fill(t || "See the attached images in my next message.") });
        if (images.length) await api.post(`/api/sessions/${sessionId}/input`, { text: t, images });
      } else {
        await api.post(`/api/sessions/${sessionId}/input`, { text: t, images });
      }
      onSent();
      setText("");
      setImages([]);
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="border-t border-line-2 px-2.5 pt-2 pb-2.5">
      <input ref={fileRef} type="file" accept="image/png,image/jpeg,image/webp,image/gif" multiple hidden onChange={(e) => { void addImages(Array.from(e.target.files ?? [])); e.target.value = ""; }} />
      {images.length > 0 && <div className="mb-2 flex flex-wrap gap-2">{images.map((image, i) => <div key={i} className="relative">
        <img src={image.data_url} alt={image.name} title={image.name} className="h-16 max-w-28 rounded-md border border-line object-contain" />
        <button className="iconbtn absolute -top-1 -right-1 bg-paper!" title={`Remove ${image.name}`} disabled={busy || reading} onClick={() => setImages((current) => current.filter((_, j) => j !== i))}><Icon name="x" size={12} /></button>
      </div>)}</div>}
      {reading && <div className="mb-1 text-[11.5px] text-ink-3">Reading images…</div>}
      <div className="flex items-end gap-2 rounded-lg border border-line px-2.5 py-[7px] focus-within:border-teal focus-within:shadow-[0_0_0_3px_var(--color-teal-soft)]">
        <textarea
          ref={ref}
          rows={1}
          value={text}
          disabled={disabled || busy || reading}
          onPaste={(e) => {
            const files = clipboardImages(e.clipboardData);
            if (files.length && acceptsImages) { e.preventDefault(); void addImages(files); }
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
        <button className="iconbtn border-line! bg-paper" title="Send" disabled={disabled || busy || reading || (!text.trim() && !images.length)} onClick={() => void send()}>
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
  return useMemo(() => {
    if (!s?.project_id || !files) return null;
    const { id, project_id, cwd } = s;
    return {
      resolve: pathResolver(files.files, cwd),
      open: (path, line) => void navigate({ to: "/projects/$id/code", params: { id: project_id }, search: { root: cwd, path, line: line ?? undefined, session: id } }),
    };
  }, [files, s?.id, s?.project_id, s?.cwd, navigate]);
}

type View = "summary" | "trace" | "changes" | "subagents";

export function SessionPane({ sessionId, onClear }: { sessionId: string; onClear: () => void }) {
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
  const [gitMain, setGitMain] = useState(false);
  const [view, setView] = useState<View | null>(null);
  useEffect(() => { if (s && view === null) setView(isRunning(s) ? "trace" : "summary"); else if (s && isRunning(s) && view === "summary") setView("trace"); }, [s, view]);
  const git = useQuery({ queryKey: ["session-git", sessionId], queryFn: () => api.get<SessionGitStatus>(`/api/sessions/${sessionId}/git`), refetchInterval: 5000 });
  const isGit = !!git.data;
  const current: View = view === "changes" && !isGit ? "summary" : view ?? "summary";
  const summary = !s || current === "summary" && !isRunning(s);
  const changesOpen = current === "changes";
  const localPr = useQuery({ queryKey: ["predrafts"], queryFn: () => api.get<import("@/gen/Predraft").Predraft[]>("/api/predrafts") }).data?.find((p) => p.root === s?.cwd || p.id === localStorage.getItem(`launchpad.session-predraft.${sessionId}`));
  const projects = useProjects().data;
  const variantName = useVariant(s?.launch_spec.variant)?.name;
  const navigate = useNavigate();
  const codeLinks = useCodeLinks(s);
  const log = useSessionLogs((st) => st.logs[sessionId]);
  const load = useSessionLogs((st) => st.load);
  useEffect(() => {
    if (!log) {
      useSessionLogs.setState((st) => ({ logs: { ...st.logs, [sessionId]: { events: [], lastSeq: 0, loaded: false, deltas: {}, outputs: {} } } }));
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
      if (r.t === "ask") return r.answers ? null : { request_id: r.request_id, count: r.questions.length };
    }
    return null;
  }, [rows]);

  const scrollRef = useRef<HTMLDivElement>(null);
  const atBottom = useRef(true);
  const traceOpen = !summary && !changesOpen && !childrenOpen;
  const v = useVirtualizer({ enabled: traceOpen, count: trace.length, getScrollElement: () => scrollRef.current, estimateSize: () => 48, overscan: 6, useFlushSync: false });
  const total = v.getTotalSize();
  useLayoutEffect(() => {
    if (atBottom.current && trace.length) v.scrollToIndex(trace.length - 1, { align: "end" });
  }, [trace.length, total, v, summary, changesOpen]);
  // Answer: show the open question in the trace and focus the message box.
  const [answering, setAnswering] = useState(0);
  useEffect(() => {
    if (!answering || !traceOpen) return;
    const i = trace.findLastIndex((r) => r.t === "ask");
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
  const changed = (git.data?.files.length ?? 0) > 0 || mainChanges > 0;
  const showChanges = (main: boolean) => { setGitMain(main); setView("changes"); };
  const onlyMain = (git.data?.files.length ?? 0) === 0 && mainChanges > 0;
  const archive = () => void act(s.id, "close");
  const placeholder =
    s.status === "in_terminal"
      ? "Open in the terminal: re-attach it to continue here"
      : s.open_question
        ? `Answer or message ${name}`
        : running
          ? harness?.working_message ?? `Message ${name}`
          : resumable
            ? `Message ${name} to resume the session`
            : `Message ${name}`;

  // The one primary action, depending on the state.
  const primary = running || s.status === "quota_paused" ? (
    <button className="btn btn-sm border-gh-closed! bg-gh-closed! text-white!" title={s.status === "quota_paused" ? "Stop: cancel automatic continuation" : "Stop: interrupt the agent"} onClick={() => void act(s.id, "interrupt")}>
      <Icon name="stop" size={12} />Stop
    </button>
  ) : s.status === "in_terminal" ? (
    tui && reattaching ? <>
      <button className="btn btn-sm btn-primary" onClick={() => { setReattaching(false); void api.post(`/api/sessions/${s.id}/resume`, { close_terminal: true }).catch(toastError); }}>Close the TUI and re-attach</button>
      <button className="btn btn-sm" onClick={() => setReattaching(false)}>Cancel</button>
    </> : <button className="btn btn-sm btn-primary" title="Continue this session in Launchpad" onClick={() => (tui ? setReattaching(true) : void act(s.id, "resume"))}>Re-attach</button>
  ) : s.status === "waiting" && s.open_question ? (
    <button className="btn btn-sm btn-primary" title="Show the question and focus the message box" onClick={() => { setView("trace"); setAnswering((n) => n + 1); }}>Answer</button>
  ) : s.status === "waiting" ? (
    changed
      ? <button className="btn btn-sm btn-primary" title="Open the changed files in the Changes view" onClick={() => showChanges(onlyMain)}>Review changes</button>
      : <button className="btn btn-sm btn-primary" title="Archive the session (it stays resumable)" onClick={archive}><Icon name="archive" size={12} />Archive</button>
  ) : resumable ? (
    <button className="btn btn-sm btn-primary" title={s.status === "closed" ? "Re-attach this archived session without sending a message" : "Send “Continue the unfinished turn.”"} onClick={() => { void (s.status === "closed" ? act(s.id, "resume") : api.post(`/api/sessions/${s.id}/input`, { text: "Continue the unfinished turn." }).catch(toastError)); setView("trace"); }}>Resume</button>
  ) : null;
  const archivePrimary = s.status === "waiting" && !s.open_question && !changed;

  return (
    <div className="relative flex h-full min-h-0 min-w-0 flex-col bg-paper"
      onDragOver={(e) => { if (e.dataTransfer.types.includes("Files")) { e.preventDefault(); setDragging(true); } }}
      onDragLeave={(e) => { if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setDragging(false); }}
      onDrop={(e) => { if (e.dataTransfer.types.includes("Files")) { e.preventDefault(); setDragging(false); dropImages.current?.(Array.from(e.dataTransfer.files)); } }}
    >
      {dragging && <div className="pointer-events-none absolute inset-0 z-10 grid place-items-center border-2 border-teal bg-teal-soft/80 font-semibold text-teal-ink">{harness?.capabilities.images ? "Drop images to attach to your message" : `${name} does not accept images`}</div>}
      <div className="grid grid-cols-[18px_1fr] items-center gap-x-[9px] gap-y-0.5 border-b border-line-2 py-2 pr-2.5 pl-3">
        <span className="order-1 row-span-2 self-start pt-px">
          <HarnessMark harness={s.harness} size={18} />
        </span>
        <div className="order-2 flex min-w-0 items-center gap-1">
          {renaming ? <div className="flex min-w-0 flex-1 flex-wrap items-center gap-1">
            <input autoFocus aria-label="Session title" className="field-input min-w-0 flex-1 basis-32 text-[12.5px] font-[650]" maxLength={100} value={title} disabled={savingTitle} onChange={(e) => setTitle(e.target.value)} />
            <button className="btn btn-sm" disabled={!title.trim() || savingTitle} onClick={async () => { setSavingTitle(true); try { await api.post(`/api/sessions/${s.id}/rename`, { title: title.trim() }); setRenaming(false); } catch (e) { toastError(e); } finally { setSavingTitle(false); } }}>Save title</button>
            <button className="btn btn-sm" disabled={savingTitle} onClick={() => setRenaming(false)}>Cancel</button>
          </div> : <button className="min-w-0 flex-1 truncate text-left text-[12.5px] font-[650] hover:text-teal-ink" title={`${s.title} · Click to edit`} aria-label={`Edit session title: ${s.title}`} onClick={() => { setTitle(s.title); setRenaming(true); }}>
            {isPinned && <span className="mr-1 text-teal-ink" title="Pinned">★</span>}{s.title}
          </button>}
          <button className="iconbtn shrink-0 self-start" title="Close panel" aria-label="Close panel" onClick={onClear}>
            <Icon name="x" size={14} />
          </button>
        </div>
        <span className="order-3 col-start-2 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-[11.5px] text-ink-3 tabular-nums">
          {localPr && <button className="text-teal-ink hover:underline" onClick={() => void navigate({ to: "/predrafts/$id", params: { id: localPr.id } })}>Pre-draft PR</button>}
          <span className="truncate">{projectName(projects, s.project_id, s.cwd)}</span>
          <WorktreeChip session={s} />
          {s.launch_spec.variant && <span title="Harness variant">{variantName ?? `${s.launch_spec.variant} (deleted variant)`}</span>}
          <StatusBadge s={s} />
          <PaidUsageToggle session={s} badge />
          <CostLimit session={s} badge />
          <span>{usageLine(s)}</span>
          <span className="ml-auto" title="Started">{new Date(s.started_at).toLocaleDateString()}</span>
        </span>
        <span className="order-4 col-span-2 flex flex-wrap items-center gap-1 pt-1">
          <span className="mr-1 inline-flex max-w-full flex-wrap gap-0.5 rounded border border-line p-0.5" aria-label="Session view">
            <button className={`btn btn-sm ${summary ? "bg-select!" : ""}`} aria-pressed={summary} disabled={running} onClick={() => setView("summary")}>Summary</button>
            <button className={`btn btn-sm ${traceOpen ? "bg-select!" : ""}`} aria-pressed={traceOpen} onClick={() => setView("trace")}>Trace</button>
            {children.length > 0 && <button className={`btn btn-sm ${childrenOpen ? "bg-select!" : ""}`} aria-pressed={childrenOpen} onClick={() => setView("subagents")}><Icon name="agents" size={12} />Subagents · {children.filter((a) => a.status === "running").length || children.length}{children.some((a) => a.status === "running") ? " running" : ""}</button>}
            {isGit && <button className={`btn btn-sm ${changesOpen ? "bg-select!" : ""}`} aria-pressed={changesOpen} title="Git changes of the session folder" onClick={() => showChanges(onlyMain)}>Changes{changed ? ` · ${git.data!.files.length || mainChanges}` : ""}</button>}
          </span>
          <span className="ml-auto flex flex-wrap items-center gap-1">
            {primary}
            <ResumeOtherAgent session={s} />
            <button className="btn btn-sm" title={`Open a shell in ${s.cwd}`} onClick={() => void openTerminal.shell(s.cwd)}>
              <Icon name="term" size={12} />
              Terminal
            </button>
            <Popover align="right" width={260} trigger={({ open, toggle }) => <button className={`btn btn-sm ${open ? "bg-select!" : ""}`} aria-expanded={open} onClick={toggle}>More<Icon name="chev" size={12} /></button>}>
              {(close) => <>
                {s.status === "in_terminal" && tui && <MenuItem label="Show TUI" detail="In the terminal dock" onClick={() => { close(); setDock({ open: true, active: tui.id }); }} />}
                {!running && s.status !== "in_terminal" && s.status !== "quota_paused" && harness?.capabilities.terminal && <MenuItem icon={<Icon name="term" size={14} />} label="Open in terminal" detail={`Continue in the ${name} TUI, outside quota protection`} onClick={() => { close(); void openTerminal.tui(s.id); }} />}
                {s.project_id && <MenuItem icon={<Icon name="file" size={14} />} label="Files" detail="Read the folder in the code viewer" onClick={() => { close(); void navigate({ to: "/projects/$id/code", params: { id: s.project_id! }, search: { root: s.cwd, session: s.id } }); }} />}
                <MenuItem icon={<Icon name="agents" size={14} />} label="Use trace as context" detail="Reference this trace in a new agent's prompt" disabled={s.last_seq === 0} onClick={() => { close(); useComposer.getState().start({ agentTraces: [{ session_id: s.id, through_seq: s.last_seq }] }); }} />
                {s.status !== "closed" && !archivePrimary && <MenuItem icon={<Icon name="archive" size={14} />} label="Archive" detail="It stays resumable" onClick={() => { close(); archive(); }} />}
                <MenuItem icon={<span className="text-teal-ink">{isPinned ? "★" : "☆"}</span>} label={isPinned ? "Unpin session" : "Pin session"} onClick={() => { close(); togglePin(s.id); }} />
              </>}
            </Popover>
          </span>
        </span>
      </div>

      {changesOpen && <GitPanel key={`${s.id}-${gitMain}`} initialMain={gitMain} sessionId={s.id} />}
      <CodeLinksContext.Provider value={codeLinks}>
      {childrenOpen && log && <SubagentsPanel key={s.id} log={log} agents={children} sessionId={s.id} live={s.live} back={() => setView("trace")} />}
      {!changesOpen && !childrenOpen && summary && (log && !log.loaded ? <div className="flex-1 p-6 text-center text-ink-3">Loading…</div> : <SessionSummary key={s.id} session={s} rows={rows} localPr={localPr} reviewDiff={showChanges} showTrace={() => setView("trace")} />)}
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
            {s.status === "starting" ? "Starting…" : "Working…"}
          </div>
        )}
      </div>
      </CodeLinksContext.Provider>
      {!childrenOpen && <MessageBox acceptsImages={!!harness?.capabilities.images} onSent={() => setView("trace")} key={s.id} dropImages={dropImages} inputRef={inputRef} sessionId={s.id} placeholder={placeholder} disabled={s.status === "in_terminal" || s.status === "quota_paused" || (running && !s.open_question && !harness?.capabilities.messages_while_running)} ask={s.open_question && s.live ? ask : null} />}
    </div>
  );
}
