// One dictation at a time in this window, whichever box started it. States: idle → starting (recording, "Connecting…")
// → listening (words arrive) → finishing (after Stop, the last words) → idle. The audio goes to Home over one
// WebSocket (`/api/voice/ws`), and Home sends it to the voice source; frames recorded before the socket or the
// source is ready wait in order (here, then on Home), so nothing said after the click is lost.

import { create } from "zustand";
import { useQuery } from "@tanstack/react-query";
import { isDemo } from "../../demo/mode";
import { api } from "../api";
import { daemonSocket, isExtension } from "../connection";
import { when } from "../format";
import type { VoiceStatus } from "@/gen/VoiceStatus";
import { micError, startCapture, type Capture } from "./capture";
import { tick, useVoiceDevice } from "./device";

export type Phase = "idle" | "starting" | "listening" | "finishing";

export type VoiceState = {
  /** The box that records (or recorded last, for its error line). */
  owner: string | null;
  phase: Phase;
  /** The words of the current phrase so far. */
  interim: string;
  level: number;
  startedAt: number;
  source: string | null;
  /** One quiet line: a failover. */
  note: string | null;
  error: string | null;
  /** The auto-stop countdown, 0 to 1 (1: stops now), or null. */
  countdown: number | null;
  /** Held (long press or Ctrl+Space): stops on release. */
  held: boolean;
};

export const useVoice = create<VoiceState>(() => ({ owner: null, phase: "idle", interim: "", level: 0, startedAt: 0, source: null, note: null, error: null, countdown: null, held: false }));

export const MAX_MS = 5 * 60_000;
const COUNTDOWN_MS = 2_000;

/** Settings and sources, from Home; null in the demo and while loading. */
export function useVoiceStatus() {
  return useQuery({ queryKey: ["voice"], queryFn: ({ signal }) => api.get<VoiceStatus>("/api/voice", signal), enabled: !isDemo && !isExtension, staleTime: 30_000, refetchInterval: 60_000 });
}

/** Whether the mic shows (voice is on), and why it is disabled (null: ready). */
export function useVoiceAvailability(): { shown: boolean; reason: string | null; status: VoiceStatus | null } {
  const status = useVoiceStatus().data ?? null;
  if (isDemo) return { shown: true, reason: "Voice input is not available in the demo", status };
  if (isExtension) return { shown: false, reason: null, status };
  if (!status) return { shown: false, reason: null, status };
  if (!status.enabled) return { shown: false, reason: status.unavailable, status };
  if (!window.isSecureContext || !navigator.mediaDevices?.getUserMedia) return { shown: true, reason: "Voice needs HTTPS. Open Launchpad through its Tailscale or Space address.", status };
  if (status.unavailable) return { shown: true, reason: status.resets_at ? `${status.unavailable} until ${when(status.resets_at * 1000)}` : status.unavailable, status };
  return { shown: true, reason: null, status };
}

// The socket to Home, opened on warm-up or start and kept while the window lives.
let socket: WebSocket | null = null;
let queue: (string | ArrayBuffer)[] = [];
let warmedAt = 0;

function ensureSocket(): WebSocket {
  if (socket && (socket.readyState === WebSocket.OPEN || socket.readyState === WebSocket.CONNECTING)) return socket;
  const ws = new WebSocket(daemonSocket("/api/voice/ws"));
  ws.binaryType = "arraybuffer";
  ws.onopen = () => { for (const m of queue) ws.send(m); queue = []; };
  ws.onmessage = (ev) => { if (typeof ev.data === "string") onMessage(JSON.parse(ev.data) as Incoming); };
  ws.onclose = () => {
    if (socket !== ws) return;
    socket = null;
    queue = [];
    const s = useVoice.getState();
    if (s.phase !== "idle") end(s.phase === "finishing" ? null : "Lost the connection to Home");
  };
  socket = ws;
  return ws;
}

function send(m: string | ArrayBuffer) {
  const ws = ensureSocket();
  if (ws.readyState === WebSocket.OPEN) ws.send(m); else queue.push(m);
}

/** The pointer is on a mic or a message box has the focus: Home connects the first source now (kept 30 s). */
export function warmVoice(keyterms: string[] = []) {
  if (isDemo || isExtension || useVoice.getState().phase !== "idle" || Date.now() - warmedAt < 25_000) return;
  warmedAt = Date.now();
  send(JSON.stringify({ type: "warm", keyterms }));
}

type Incoming = { type: "source"; name: string; note?: string } | { type: "delta"; text: string } | { type: "done"; text: string } | { type: "error"; reason: string } | { type: "closed" };

type Target = { owner: string; insert: (phrase: string) => void; keyterms: string[]; profile?: string; silenceMs: number };
let target: Target | null = null;
let capture: Capture | null = null;
let finished: (() => void)[] = [];
let quietSince = 0, floor = 0.01, heardAt = 0;
let limitTimer: ReturnType<typeof setTimeout> | null = null;

function onMessage(m: Incoming) {
  const s = useVoice.getState();
  if (s.phase === "idle") return;
  if (m.type === "source") useVoice.setState({ source: m.name, note: m.note ?? null, phase: s.phase === "starting" ? "listening" : s.phase });
  else if (m.type === "delta") useVoice.setState({ interim: m.text });
  else if (m.type === "done") { useVoice.setState({ interim: "" }); target?.insert(m.text); }
  else if (m.type === "error") end(m.reason);
  else if (m.type === "closed") end(null);
}

function end(error: string | null) {
  capture?.stop();
  capture = null;
  if (limitTimer) clearTimeout(limitTimer);
  const s = useVoice.getState();
  if (s.phase !== "idle" && !error) tick("stop");
  // An unsettled phrase still lands: the provider never sent its end.
  if (s.interim && target) target.insert(s.interim);
  useVoice.setState({ phase: "idle", interim: "", level: 0, countdown: null, held: false, error, ...(error ? {} : { note: null }) });
  target = null;
  const waiting = finished;
  finished = [];
  waiting.forEach((f) => f());
}

function onFrame(pcm: ArrayBuffer, level: number) {
  const s = useVoice.getState();
  if (s.phase === "idle" || s.phase === "finishing") return;
  send(pcm);
  // Speech is well above the room's noise floor, which rises slowly and falls at once.
  floor = Math.min(Math.max(floor * 1.002, 0.002), Math.max(level, 0.002));
  const now = Date.now();
  if (level > Math.max(0.015, floor * 3)) { heardAt = now; quietSince = 0; } else if (!quietSince) quietSince = now;
  let countdown: number | null = null;
  const silence = target?.silenceMs ?? 0;
  if (silence > 0 && !s.held && quietSince) {
    const quiet = now - Math.max(quietSince, s.startedAt);
    if (quiet >= silence) { void stopDictation(); return; }
    if (quiet >= silence - COUNTDOWN_MS) countdown = (quiet - (silence - COUNTDOWN_MS)) / COUNTDOWN_MS;
  }
  useVoice.setState({ level: Math.min(1, level * 6), countdown });
}

/** Starts dictating into a box. A dictation in another box stops first (its text stays there). */
export async function startDictation(t: Omit<Target, "silenceMs">, opts: { held?: boolean; silenceS?: number } = {}) {
  if (isDemo) return;
  const s = useVoice.getState();
  if (s.phase !== "idle") await stopDictation();
  target = { ...t, silenceMs: (opts.silenceS ?? 8) * 1000 };
  quietSince = 0; heardAt = 0; floor = 0.01;
  useVoice.setState({ owner: t.owner, phase: "starting", interim: "", level: 0, startedAt: Date.now(), source: null, note: null, error: null, countdown: null, held: !!opts.held });
  tick("start");
  send(JSON.stringify({ type: "start", keyterms: t.keyterms, ...(t.profile ? { profile: t.profile } : {}) }));
  try {
    const c = await startCapture(useVoiceDevice.getState().mic, onFrame);
    if (useVoice.getState().phase === "idle" || target?.owner !== t.owner) { c.stop(); return; }
    capture = c;
  } catch (e) {
    send(JSON.stringify({ type: "cancel" }));
    end(micError(e));
    return;
  }
  limitTimer = setTimeout(() => void stopDictation(), MAX_MS);
}

/** Stops recording; resolves once the last words are in the box (or after 7 s). */
export function stopDictation(): Promise<void> {
  const s = useVoice.getState();
  if (s.phase === "idle") return Promise.resolve();
  const done = new Promise<void>((resolve) => finished.push(resolve));
  if (s.phase !== "finishing") {
    capture?.stop();
    capture = null;
    useVoice.setState({ phase: "finishing", countdown: null, held: false });
    send(JSON.stringify({ type: "stop" }));
    setTimeout(() => { if (useVoice.getState().phase === "finishing") { send(JSON.stringify({ type: "cancel" })); end(null); } }, 7_000);
  }
  return done;
}

/** Drops the dictation (its settled phrases stay in the box). */
export function cancelDictation() {
  if (useVoice.getState().phase === "idle") return;
  send(JSON.stringify({ type: "cancel" }));
  useVoice.setState({ interim: "" });
  end(null);
}

export function clearVoiceError(owner: string) {
  if (useVoice.getState().owner === owner) useVoice.setState({ error: null, note: null });
}

/** When the first words came, for Test voice (0: none yet). */
export const firstHeard = () => heardAt;
