// The message box of an agent session: typed messages (with presets, context and attachments) or typed answers to
// its open question. Used by the session page and by Today's finished agents (steer an agent without opening it).

import { useLayoutEffect, useRef, useState } from "react";
import { Icon } from "@/components/Icon";
import { VoiceButton } from "@/components/VoiceButton";
import { VoiceCaption } from "@/components/VoiceCaption";
import { usePhone } from "@/lib/phone";
import { useDictation } from "@/lib/voice/useDictation";
import { toastError } from "@/components/Toast";
import { PromptField } from "@/features/launch/PromptField";
import { api, ApiError } from "@/lib/api";
import { pastedImages } from "@/lib/attachments";
import { abandonOperation } from "@/lib/operations";
import { withPresets } from "@/lib/presets";
import { useProjects, useSession } from "@/lib/queries";
import { useMessageDrafts } from "@/stores/messages";
import { ATTACH_LIMITS, AttachmentList, useMessageAttachments } from "./MessageAttachments";
import { EMPTY_CONTEXT, hasMessageContext, MessageContextButton, MessageContextChips } from "./MessageContext";
import { PresetChips, usePresets } from "./MessagePresets";

export type PendingAsk = { request_id: string; count: number; permission: boolean } | null;

/** The typed answers to `count` questions: one non-empty line each, else null. */
export function typedAnswers(text: string, count: number): string[] | null {
  const lines = text.split("\n").map((l) => l.trim()).filter(Boolean);
  return lines.length === count ? lines : null;
}

/** The session message box (on the phone, opened by Reply in the bottom bar and closed by `onHide`), also under a
 * finished agent's row in Today (`className` replaces its frame). Drafts are kept per session in `useMessageDrafts`. */
export function MessageBox({ acceptsImages, sessionId, placeholder, disabled, ask, dropImages, inputRef: ref, onSent, onHide, className = "border-t border-line-2 px-2.5 pt-2 pb-2.5" }: { acceptsImages: boolean; sessionId: string; placeholder: string; disabled: boolean; ask: PendingAsk; dropImages: React.RefObject<((files: File[]) => void) | null>; inputRef: React.RefObject<HTMLTextAreaElement | null>; onSent: () => void; onHide?: () => void; className?: string }) {
  const promptSession = useSession(sessionId);
  const promptProjects = useProjects().data ?? [];
  const promptProject = promptProjects.find((p) => p.id === promptSession?.project_id);
  const setDocument = useMessageDrafts((s) => s.setDocument);
  const text = useMessageDrafts((s) => s.drafts[sessionId] ?? "");
  const setDraft = useMessageDrafts((s) => s.set);
  const setText = (t: string) => setDraft(sessionId, t);
  const presetSurface = promptSession?.launch_spec.bot ? "bots" : "agent";
  const allPresets = usePresets(presetSurface);
  const selectedPresets = useMessageDrafts((s) => s.presets[sessionId]);
  const setStoredPresets = useMessageDrafts((s) => s.setPresets);
  // Presets go with messages, not with answers to an open question.
  const presets = ask ? [] : (selectedPresets ?? []).filter((id) => allPresets.some((p) => p.id === id));
  const setPresets = (ids: string[]) => setStoredPresets(sessionId, ids);
  const storedContext = useMessageDrafts((s) => s.contexts[sessionId]);
  const context = ask ? EMPTY_CONTEXT : storedContext ?? EMPTY_CONTEXT;
  const setStoredContext = useMessageDrafts((s) => s.setContext);
  const setContext = (value: typeof context) => setStoredContext(sessionId, value);
  const message = ask ? text.trim() : withPresets(text, presets, allPresets);
  const [busy, setBusy] = useState(false);
  const [delivery, setDelivery] = useState<"sending" | "delivered" | "uncertain" | "failed" | null>(null);
  const attachments = useMessageAttachments(acceptsImages);
  const { files, reading, readingRef } = attachments;
  const payload = { text: message, files, ...(hasMessageContext(context) ? { context } : {}) };
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
  const phone = usePhone();
  // Phone Reply: dictating closes the keyboard (the text stays in view over a large Stop); back stops and keeps the text.
  const dictation = useDictation({ inputRef: ref, onSend: () => void send(), blurOnStart: !!onHide, keep: { read: () => useMessageDrafts.getState().drafts[sessionId] ?? "", write: (t) => useMessageDrafts.getState().set(sessionId, t) } });
  const bigStop = phone && !!onHide && dictation.active;
  const send = async () => {
    const t = message;
    if ((!t && !files.length && !hasMessageContext(context)) || busy || disabled || reading || ask?.permission) return;
    setBusy(true);
    setDelivery("sending");
    try {
      if (ask) {
        // A typed answer to the open question.
        // Several questions: one typed line per question, in order (never the same text for every question).
        const answers = ask.count > 1 ? typedAnswers(t, ask.count) : [t || "See the attachments in my next message."];
        if (!answers) {
          toastError(new Error(`The agent asked ${ask.count} questions: type one answer per line, in order, or pick the options in the question card.`));
          setDelivery(null);
          return;
        }
        await api.post(`/api/sessions/${sessionId}/answer`, { request_id: ask.request_id, answers });
        if (files.length) await api.post(`/api/sessions/${sessionId}/input`, payload);
      } else {
        await api.post(`/api/sessions/${sessionId}/input`, payload);
      }
      setDelivery("delivered");
      onSent();
      setText("");
      if (!ask) { setPresets([]); setContext(EMPTY_CONTEXT); }
      attachments.clear();
    } catch (e) {
      setDelivery(e instanceof ApiError && e.operationCompleted ? "failed" : "uncertain");
      toastError(e);
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className={className} data-tour="session.reply">
      <input ref={fileRef} type="file" multiple hidden onChange={(e) => { addFiles(Array.from(e.target.files ?? [])); e.target.value = ""; }} />
      <AttachmentList files={files} disabled={busy || reading} onRemove={attachments.remove} />
      {delivery && <div role="status" className="mb-1 text-[12px] text-ink-3">{delivery === "sending" ? "Sending…" : delivery === "delivered" ? promptSession?.quota_pause ? "Queued for when the agent resumes" : "Delivered" : delivery === "failed" ? "Attempt failed. Check the trace before starting a new attempt." : "Delivery not confirmed. Retry the same message to check its result."}</div>}
      {delivery === "uncertain" && <button className="btn btn-sm mb-1" disabled={busy} onClick={() => void abandonOperation(`/api/sessions/${sessionId}/input`, payload).then(() => setDelivery(null)).catch(toastError)}>I checked the trace — start a new attempt</button>}
      {reading && <div className="mb-1 text-[12px] text-ink-3">Reading attachments…</div>}
      <VoiceCaption dictation={dictation} />
      {onHide ? (
        <div className="mb-1.5 flex items-start gap-2">
          {!ask && <PresetChips surface={presetSurface} selected={presets} onChange={setPresets} disabled={busy} className="min-w-0 flex-1" />}
          <button className="btn btn-sm btn-ghost ml-auto" title="Hide the message box (it keeps your draft)" onClick={onHide}>Hide</button>
        </div>
      ) : !ask && <PresetChips surface={presetSurface} selected={presets} onChange={setPresets} disabled={busy} className="mb-1.5" />}
      <div className="session-message-input flex items-end gap-2 rounded-control border border-line px-2.5 py-[7px] focus-within:border-teal focus-within:shadow-[0_0_0_3px_var(--color-teal-soft)]">
        <PromptField
          compact inputRef={ref}
          document={{ ...EMPTY_CONTEXT, ...context, text }}
          onDocumentChange={(doc) => ask ? setText(doc.text) : setDocument(sessionId, doc)}
          root={promptSession?.cwd ?? null} harness={promptSession?.harness ?? null} profile={promptSession?.launch_spec.profile ?? null}
          repo={promptProject?.gh_repo ?? promptProject?.gh_upstream ?? null} projectId={promptSession?.project_id} sessionId={sessionId}
          noFolders={!!promptSession && promptSession.launch_spec.target.kind === "browser" && !promptSession.launch_spec.target.project_id}
          autocomplete={!ask && !!promptSession} onSend={() => void send()}
          disabled={disabled || busy || reading}
          onPaste={(e) => {
            if (!acceptsImages || disabled || busy || readingRef.current) return;
            const pasted = pastedImages(e);
            if (pasted) addFiles(pasted);
          }}
          placeholder={placeholder}
        />
        {!ask && <MessageContextButton sessionId={sessionId} value={context} onChange={setContext} presets={{ presets: allPresets, selected: presets, onChange: setPresets }} disabled={disabled || busy || reading} />}
        <button className="iconbtn" title={`Attach files or images (you can also drop files${acceptsImages ? " or paste a screenshot" : ""}). ${ATTACH_LIMITS}`} aria-label="Attach files or images" disabled={disabled || busy || reading} onClick={() => fileRef.current?.click()}>
          <Icon name="clip" size={14} />
        </button>
        <VoiceButton dictation={dictation} className={bigStop ? "hidden" : ""} tour="session.mic" />
        {dictation.active && !bigStop ? <button className="btn btn-sm flex-none" title="Finish the dictation, then send" disabled={disabled || busy || reading || dictation.phase === "finishing"} onClick={dictation.stopAndSend}>Stop &amp; send</button> : <button className="session-send iconbtn border-line! bg-paper" aria-label="Send" title={delivery === "failed" ? "Send as a new attempt" : delivery === "uncertain" ? "Retry delivery" : promptSession?.quota_pause ? "Queue for resume" : "Send"} disabled={disabled || busy || reading || (!message && !files.length && !hasMessageContext(context))} onClick={() => void send()}>
          <Icon name="send" size={14} />
        </button>}
      </div>
      {bigStop && <div className="mt-2 flex gap-2">
        <VoiceButton dictation={dictation} large />
        <button className="btn h-[52px] px-5" disabled={disabled || busy || reading || dictation.phase === "finishing"} onClick={dictation.stopAndSend}><Icon name="send" size={14} />Send</button>
      </div>}
      {!ask && <MessageContextChips value={context} onChange={setContext} disabled={disabled || busy || reading} />}
    </div>
  );
}
