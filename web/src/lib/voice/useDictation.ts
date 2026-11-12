// A message box's dictation: the mic's state for this box, insertion at its caret, Stop & send, warm-up on focus and
// the keyboard: Ctrl+Space in the box (a tap toggles, holding ≥ 300 ms talks until release) and Esc stops.

import { useEffect, useId, useRef } from "react";
import { useBots } from "../bots";
import { useScope } from "../workspace";
import { usePhone } from "../phone";
import { cancelDictation, startDictation, stopDictation, useVoice, useVoiceAvailability, warmVoice, type Phase } from "./dictation";
import { insertAtCaret } from "./text";

export type Dictation = {
  id: string;
  /** This box's phase (idle while another box records). */
  phase: Phase;
  active: boolean;
  shown: boolean;
  /** Why the mic is disabled, or null. */
  reason: string | null;
  toggle: () => void;
  start: (held?: boolean) => void;
  stop: () => Promise<void>;
  /** Stop, then the box's send once the last words are in. */
  stopAndSend: () => void;
  cancel: () => void;
  warm: () => void;
};

const HOLD_MS = 300;

/** The names agents and bots of this workspace go by, as vocabulary hints. */
function useTerms(extra: string[] | undefined): string[] {
  const scope = useScope();
  const bots = useBots().data ?? [];
  return [...(extra ?? []), ...bots.filter((b) => b.workspace_id === scope.workspace.id).map((b) => b.settings.name), ...scope.projects.map((p) => p.name)].slice(0, 80);
}

export function useDictation({ inputRef, onSend, map, keyterms, blurOnStart, profile }: {
  inputRef: React.RefObject<HTMLTextAreaElement | HTMLInputElement | null>;
  /** The box's send, called by Stop & send after the last words are in. */
  onSend?: () => void;
  /** Turns a settled phrase into the box's text (spoken bot names into mentions in channels). */
  map?: (phrase: string) => string;
  keyterms?: string[];
  /** Phone Reply: the keyboard closes while dictating, so the text stays in view. */
  blurOnStart?: boolean;
  /** Test voice: only this profile. */
  profile?: string;
}): Dictation {
  const id = useId();
  const phone = usePhone();
  const { shown, reason, status } = useVoiceAvailability();
  const mine = useVoice((s) => s.owner === id);
  const phase = useVoice((s) => (s.owner === id ? s.phase : "idle"));
  const terms = useTerms(keyterms);
  const latest = useRef({ onSend, map, terms, silence: status?.silence_stop_s ?? 8 });
  latest.current = { onSend, map, terms, silence: status?.silence_stop_s ?? 8 };
  const ready = shown && !reason;
  const start = (held = false) => {
    if (!ready) return;
    if (blurOnStart || phone) inputRef.current?.blur();
    void startDictation({
      owner: id,
      keyterms: latest.current.terms,
      profile,
      insert: (phrase) => {
        const el = inputRef.current;
        if (!el) return;
        insertAtCaret(el, latest.current.map ? latest.current.map(phrase) : phrase);
      },
    }, { held, silenceS: latest.current.silence });
  };
  const stop = () => (useVoice.getState().owner === id ? stopDictation() : Promise.resolve());
  const active = phase !== "idle";
  const toggle = () => { if (active) void stop(); else start(); };
  const stopAndSend = () => {
    void stop().then(() => setTimeout(() => latest.current.onSend?.(), 30));
  };
  const warm = () => { if (ready) warmVoice(latest.current.terms); };
  // The keyboard, in this box only.
  const handlers = useRef({ start, stop, warm, active });
  handlers.current = { start, stop, warm, active };
  useEffect(() => {
    const el = inputRef.current;
    if (!el || !ready) return;
    let pressedAt = 0, startedByPress = false;
    const down = (e: Event) => {
      const k = e as KeyboardEvent;
      if (k.key === "Escape" && handlers.current.active) { k.preventDefault(); k.stopPropagation(); void handlers.current.stop(); return; }
      if (k.code !== "Space" || !k.ctrlKey || k.metaKey || k.altKey || k.shiftKey) return;
      k.preventDefault();
      if (k.repeat) return;
      pressedAt = Date.now();
      startedByPress = !handlers.current.active;
      if (startedByPress) handlers.current.start(true);
    };
    const up = (e: Event) => {
      const k = e as KeyboardEvent;
      if (!pressedAt || (k.code !== "Space" && k.key !== "Control")) return;
      const held = Date.now() - pressedAt >= HOLD_MS;
      pressedAt = 0;
      if (startedByPress) {
        // A tap keeps recording (toggle); a hold talks until release.
        if (held) void handlers.current.stop(); else useVoice.setState({ held: false });
      } else void handlers.current.stop();
    };
    const focus = () => handlers.current.warm();
    el.addEventListener("keydown", down);
    el.addEventListener("keyup", up);
    el.addEventListener("focus", focus);
    return () => { el.removeEventListener("keydown", down); el.removeEventListener("keyup", up); el.removeEventListener("focus", focus); };
  }, [inputRef, ready]);
  // Esc anywhere stops this box's dictation (the phone's field is blurred while dictating).
  useEffect(() => {
    if (!active) return;
    const esc = (e: KeyboardEvent) => { if (e.key === "Escape") { e.preventDefault(); void stopDictation(); } };
    window.addEventListener("keydown", esc);
    return () => window.removeEventListener("keydown", esc);
  }, [active]);
  // A box that goes away stops its dictation; its text stays in the draft.
  useEffect(() => () => { if (useVoice.getState().owner === id) void stopDictation(); }, [id]);
  return { id, phase, active: active && mine, shown, reason, toggle, start, stop, stopAndSend, cancel: () => { if (mine) cancelDictation(); }, warm };
}
