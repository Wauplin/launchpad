import { useRemote } from "@/lib/remote";
// One session in a pane: header with state and actions, the conversation, and the message box.
// The conversation is virtualized (spike 4 rules: memoized rows, no flushSync).

import { GitPanel } from "./GitPanel";
import { useLayout } from "@/stores/layout";
import { useQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import { lazy, Suspense, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { ImageAttachment } from "@/gen/ImageAttachment";
import { HarnessMark } from "@/components/HarnessMark";
import { CodeLinksContext, type CodeLinks } from "@/components/Markdown";
import { pathResolver, useCodeFiles } from "@/features/code/lib";
import { useMessageDrafts } from "@/stores/messages";
import { isRunning } from "@/lib/sessionStatus";
import { Icon } from "@/components/Icon";
import { toastError } from "@/components/Toast";
import { api } from "@/lib/api";
import { projectName, useProjects, useSession } from "@/lib/queries";
import { useSessionLogs } from "@/lib/sessionStore";
import { buildRows } from "./rows";
import { EventRow } from "./EventRow";
import { StatusBadge, usageLine } from "./SessionBits";

const EmbeddedTerminal = lazy(() => import("./EmbeddedTerminal").then((m) => ({ default: m.EmbeddedTerminal })));

function act(id: string, action: string) {
  return api.post(`/api/sessions/${id}/${action}`).catch(toastError);
}

type PendingAsk = { request_id: string; count: number } | null;

function MessageBox({ sessionId, placeholder, disabled, ask, dropImages }: { sessionId: string; placeholder: string; disabled: boolean; ask: PendingAsk; dropImages: React.RefObject<((files: File[]) => void) | null> }) {
  const text = useMessageDrafts((s) => s.drafts[sessionId] ?? "");
  const setDraft = useMessageDrafts((s) => s.set);
  const setText = (t: string) => setDraft(sessionId, t);
  const [busy, setBusy] = useState(false);
  const [images, setImages] = useState<ImageAttachment[]>([]);
  const [reading, setReading] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);
  const addImages = async (files: File[]) => {
    if (disabled || busy || reading) return;
    setReading(true);
    try {
      if (images.length + files.length > 8) throw new Error("Attach at most 8 images.");
      const added = await Promise.all(files.map(async (file) => {
        if (!["image/png", "image/jpeg", "image/webp", "image/gif"].includes(file.type)) throw new Error("Use PNG, JPEG, WebP or GIF images.");
        if (file.size > 5 * 1024 * 1024) throw new Error("Each image must be 5 MB or smaller.");
        const data_url = await new Promise<string>((resolve, reject) => {
          const reader = new FileReader();
          reader.onload = () => resolve(String(reader.result));
          reader.onerror = () => reject(new Error("Could not read the image."));
          reader.readAsDataURL(file);
        });
        return { name: file.name || "Pasted image.png", data_url };
      }));
      if ([...images, ...added].reduce((n, i) => n + i.data_url.length, 0) > 28_000_000) throw new Error("Images must total 20 MB or less.");
      setImages((current) => [...current, ...added]);
    } catch (e) { toastError(e); }
    finally { setReading(false); }
  };
  useLayoutEffect(() => { dropImages.current = (files) => { void addImages(files); }; });
  const ref = useRef<HTMLTextAreaElement>(null);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 160)}px`;
  }, [text]);
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
      <div className="mb-1 flex items-center gap-2 text-[11.5px] text-ink-3">
        <button className="btn btn-sm" disabled={disabled || busy || reading} onClick={() => fileRef.current?.click()}>Attach images</button>
        <span>{reading ? "Reading images…" : "Drop images here or paste into the message box"}</span>
      </div>
      <div className="flex items-end gap-2 rounded-lg border border-line px-2.5 py-[7px] focus-within:border-teal focus-within:shadow-[0_0_0_3px_var(--color-teal-soft)]">
        <textarea
          ref={ref}
          rows={1}
          value={text}
          disabled={disabled || busy || reading}
          onPaste={(e) => {
            const files = Array.from(e.clipboardData.items).filter((item) => item.kind === "file" && item.type.startsWith("image/")).map((item) => item.getAsFile()).filter((file): file is File => !!file);
            if (files.length) { e.preventDefault(); void addImages(files); }
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
        <button className="iconbtn border-line! bg-paper" title="Send" disabled={disabled || busy || reading || (!text.trim() && !images.length)} onClick={() => void send()}>
          <Icon name="send" size={14} />
        </button>
      </div>
    </div>
  );
}

/** File paths in the conversation open the code viewer on the session's folder. */
function useCodeLinks(s: { project_id: string | null; cwd: string } | null): CodeLinks | null {
  const navigate = useNavigate();
  const files = useCodeFiles(s?.project_id ? s.cwd : null, null).data;
  return useMemo(() => {
    if (!s?.project_id || !files) return null;
    const { project_id, cwd } = s;
    return {
      resolve: pathResolver(files.files, cwd),
      open: (path, line) => void navigate({ to: "/projects/$id/code", params: { id: project_id }, search: { root: cwd, path, line: line ?? undefined } }),
    };
  }, [files, s?.project_id, s?.cwd, navigate]);
}

export function SessionPane({ sessionId, focused, onFocus, onClear }: { sessionId: string; focused: boolean; onFocus: () => void; onClear: () => void }) {
  const remote = useRemote((s) => s.enabled);
  const [terminalOpen, setTerminalOpen] = useState(false);
  const [terminalBusy, setTerminalBusy] = useState(false);
  const [gitOpen, setGitOpen] = useState(false);
  const [renaming, setRenaming] = useState(false);
  const [title, setTitle] = useState("");
  const [savingTitle, setSavingTitle] = useState(false);
  const { pinned, togglePin, layout, panes, place } = useLayout();
  const dropImages = useRef<((files: File[]) => void) | null>(null);
  const [dragging, setDragging] = useState(false);
  const s = useSession(sessionId);
  const terminalState = useQuery({ queryKey: ["terminal", sessionId, s?.status], queryFn: () => api.get<{running:boolean;output:string}>(`/api/sessions/${sessionId}/terminal/snapshot`), enabled: s?.status === "in_terminal", retry: false, refetchInterval: s?.status === "in_terminal" ? 3000 : false });
  const embedded = s?.status === "in_terminal" && !!terminalState.data && (terminalState.data.running || !!terminalState.data.output);
  useEffect(() => { if (embedded) setTerminalOpen(true); }, [embedded]);
  const localPr = useQuery({ queryKey: ["predrafts"], queryFn: () => api.get<import("@/gen/Predraft").Predraft[]>("/api/predrafts") }).data?.find((p) => p.root === s?.cwd);
  const projects = useProjects().data;
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
  const ask: PendingAsk = useMemo(() => {
    for (let i = rows.length - 1; i >= 0; i--) {
      const r = rows[i];
      if (r.t === "ask") return r.answers ? null : { request_id: r.request_id, count: r.questions.length };
    }
    return null;
  }, [rows]);

  const scrollRef = useRef<HTMLDivElement>(null);
  const atBottom = useRef(true);
  const v = useVirtualizer({ count: rows.length, getScrollElement: () => scrollRef.current, estimateSize: () => 48, overscan: 6, useFlushSync: false });
  const total = v.getTotalSize();
  useLayoutEffect(() => {
    if (atBottom.current && rows.length) v.scrollToIndex(rows.length - 1, { align: "end" });
  }, [rows.length, total, v]);
  // Re-render every 20 s for elapsed times.
  const [, tick] = useState(0);
  useEffect(() => {
    const t = setInterval(() => tick((n) => n + 1), 20_000);
    return () => clearInterval(t);
  }, []);

  if (!s) {
    return (
      <div className="grid h-full place-items-center text-ink-3" onMouseDown={onFocus}>
        This session no longer exists.
      </div>
    );
  }
  const running = isRunning(s);
  const resumable = s.status === "closed" || s.status === "interrupted" || s.status === "error";
  const name = s.harness === "claude" ? "Claude" : "Codex";
  const placeholder =
    s.status === "in_terminal"
      ? "Open in the terminal: re-attach it to continue here"
      : s.open_question
        ? `Answer or message ${name}`
        : running
          ? s.harness === "claude"
            ? "Message Claude. It is read after the current step."
            : "Message Codex. It steers the running turn."
          : resumable
            ? `Message ${name} to resume the session`
            : `Message ${name}`;

  return (
    <div className={`relative flex h-full min-h-0 min-w-0 flex-col bg-paper ${focused ? "z-[1] shadow-[inset_0_0_0_2px_var(--color-teal)]" : ""}`} onMouseDown={onFocus}
      onDragOver={(e) => { if (e.dataTransfer.types.includes("Files")) { e.preventDefault(); setDragging(true); } }}
      onDragLeave={(e) => { if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setDragging(false); }}
      onDrop={(e) => { if (e.dataTransfer.types.includes("Files")) { e.preventDefault(); setDragging(false); dropImages.current?.(Array.from(e.dataTransfer.files)); } }}
    >
      {dragging && <div className="pointer-events-none absolute inset-0 z-10 grid place-items-center border-2 border-teal bg-teal-soft/80 font-semibold text-teal-ink">Drop images to attach to your message</div>}
      <div className="grid grid-cols-[18px_1fr] items-center gap-x-[9px] gap-y-0.5 border-b border-line-2 py-2 pr-2.5 pl-3">
        <span className="order-1 row-span-2 self-start pt-px">
          <HarnessMark harness={s.harness} size={18} />
        </span>
        <span draggable onDragStart={(e) => { e.dataTransfer.setData("application/x-launchpad-session",s.id); e.dataTransfer.effectAllowed = "move"; }} className="order-2 cursor-grab truncate text-[12.5px] font-[650]" title={`${s.title} · Drag to another pane`}>
          {s.title}
        </span>
        <span className="order-4 col-span-2 flex flex-wrap gap-0.5 self-start pt-1">
          {running && (
            <button className="iconbtn" title="Stop: interrupt the agent" onClick={() => void act(s.id, "interrupt")}>
              <Icon name="stop" size={14} />
            </button>
          )}
          {s.status === "in_terminal" ? (
            <button className="btn btn-sm" disabled={embedded} title={embedded ? "Use Stop TUI and re-attach in the terminal panel" : "Continue this session in Launchpad"} onClick={() => void act(s.id, "resume")}>
              Re-attach
            </button>
          ) : (
            !running && (
              <button className="iconbtn" title={remote ? "Copy terminal command" : "Open in terminal"} onClick={() => void act(s.id, "open-terminal")}>
                <Icon name="term" size={14} />
              </button>
            )
          )}
          {!running && s.status !== "in_terminal" && <button className="btn btn-sm" disabled={terminalBusy || s.open_question || !s.harness_session_id} onClick={async () => {
            setTerminalBusy(true);
            try { await api.post(`/api/sessions/${s.id}/embedded-terminal`); setTerminalOpen(true); } catch(e) { toastError(e); } finally { setTerminalBusy(false); }
          }}>{terminalBusy ? "Starting TUI…" : "Embedded terminal"}</button>}
          {embedded && <button className="btn btn-sm" onClick={() => setTerminalOpen(!terminalOpen)}>{terminalOpen ? "Conversation" : "Show terminal"}</button>}
          {s.project_id && (
            <button
              className="btn btn-sm"
              title={`Read the files of ${s.cwd} in the code viewer`}
              onClick={() => void navigate({ to: "/projects/$id/code", params: { id: s.project_id! }, search: { root: s.cwd } })}
            >
              Files
            </button>
          )}
          {resumable && (
            <button className="btn btn-sm" title="Resume the session" onClick={() => void act(s.id, "resume")}>
              Resume
            </button>
          )}
          {s.status !== "closed" && (
            <button className="btn btn-sm" title="Archive the session (it stays resumable)" onClick={() => void act(s.id, "close")}>
              Archive
            </button>
          )}
          <button className="iconbtn" title="Close panel" onClick={onClear}>
            <Icon name="x" size={14} />
          </button>
        </span>
        <span className="order-3 col-start-2 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-[11.5px] text-ink-3 tabular-nums">
          {localPr && <button className="text-teal-ink hover:underline" onClick={() => void navigate({ to: "/predrafts/$id", params: { id: localPr.id } })}>Pre-draft PR</button>}
        <span className="truncate">{projectName(projects, s.project_id, s.cwd)}</span>
          <StatusBadge s={s} />
          <span>{usageLine(s)}</span>
        </span>
      </div>

      <div className="flex flex-wrap items-center gap-1 border-b border-line-2 px-2 py-1 text-xs">
        <button className="btn btn-sm" aria-pressed={pinned.includes(s.id)} onClick={() => togglePin(s.id)}>{pinned.includes(s.id) ? "★ Unpin" : "☆ Pin"}</button>
        <button className="btn btn-sm" onClick={() => { setTitle(s.title); setRenaming(!renaming); }}>Rename</button>
        <button className={`btn btn-sm ${gitOpen ? "bg-select!" : ""}`} aria-pressed={gitOpen} onClick={() => setGitOpen(!gitOpen)}>Git</button>
        <button className="btn btn-sm" onClick={() => void api.post("/api/open-editor", { path:s.cwd }).catch(toastError)}>{remote ? "Copy editor command" : "Open in editor"}</button>
        {layout > 1 && <select aria-label="Move session to pane" className="field-input max-w-32" value="" onChange={(e) => place(s.id,Number(e.target.value))}><option value="">Move to pane…</option>{Array.from({length:layout},(_,i) => panes[i] !== s.id && <option key={i} value={i}>Pane {i+1}</option>)}</select>}
        <span className="ml-auto text-ink-3">{new Date(s.started_at).toLocaleDateString()}</span>
      </div>
      {renaming && <div className="flex gap-2 border-b border-line p-2"><input aria-label="Session title" className="field-input min-w-0 flex-1" maxLength={100} value={title} onChange={(e) => setTitle(e.target.value)} /><button className="btn btn-sm" disabled={!title.trim() || savingTitle} onClick={async () => { setSavingTitle(true); try { await api.post(`/api/sessions/${s.id}/rename`, {title}); setRenaming(false); } catch(e) { toastError(e); } finally { setSavingTitle(false); } }}>Save title</button><button className="btn btn-sm" onClick={() => setRenaming(false)}>Cancel</button></div>}
      {gitOpen && !terminalOpen && <GitPanel sessionId={s.id} />}
      {terminalOpen && s.status === "in_terminal" ? <Suspense fallback={<div className="p-4 text-xs text-ink-3">Loading terminal…</div>}><EmbeddedTerminal sessionId={s.id} onReturn={() => { setTerminalOpen(false); void terminalState.refetch(); }} /></Suspense> : <>
      <CodeLinksContext.Provider value={codeLinks}>
      <div
        ref={scrollRef}
        className={`min-h-0 flex-1 overflow-y-auto px-3 ${gitOpen ? "hidden" : ""}`}
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
                key={rows[it.index].key}
                data-index={it.index}
                ref={v.measureElement}
                className="flex flex-col py-[5px]"
                style={{ position: "absolute", top: 0, left: 0, right: 0, transform: `translateY(${it.start}px)` }}
              >
                <EventRow row={rows[it.index]} sessionId={s.id} live={s.live} />
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
      <MessageBox key={s.id} dropImages={dropImages} sessionId={s.id} placeholder={placeholder} disabled={s.status === "in_terminal"} ask={s.open_question && s.live ? ask : null} />
      </>}
    </div>
  );
}
