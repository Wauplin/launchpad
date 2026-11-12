// Demo voice: Settings → Voice lists the demo's profiles as sources (`GET /api/voice`, like `crates/server/src/voice.rs`),
// and a dictation plays a scripted voice through the real UI states. The dictation socket answers like Home
// (source, interim words, settled phrases, done), and the capture plays speech-like levels without a microphone,
// so nothing is recorded, sent to a daemon or written to browser storage.

import type { VoiceSource } from "@/gen/VoiceSource";
import type { VoiceStatus } from "@/gen/VoiceStatus";
import { replaceCapture } from "@/lib/voice/capture";
import { clock, onVoiceSocket, route, type FakeSocket } from "../daemon";
import { world } from "../world";

/** The demo's second Claude login has used up its weekly window: the source picker skips it. */
const USED_UP = new Set(["claude-pro"]);
const resetsAt = () => Math.floor(clock.ago(-60 * 26) / 1000);

function sources(): VoiceStatus {
  const { profiles, voice } = world.config;
  const harness = (id: string) => world.harnesses.find((h) => h.id === id);
  const eligible = (p: (typeof profiles)[number]) => !!harness(p.harness)?.capabilities.voice && p.billing === "subscription";
  const order = voice.profiles.length ? voice.profiles.flatMap((id) => profiles.filter((p) => p.id === id && eligible(p))) : profiles.filter(eligible);
  const used = order.length;
  const all = [...order, ...profiles.filter((p) => !order.includes(p))];
  let until: number | null = null;
  const out = all.map((p, i): VoiceSource => {
    const h = harness(p.harness);
    const base = { profile: p.id, name: p.name, harness: h?.name ?? p.harness, used: i < used, resets_at: null };
    if (!h?.capabilities.voice) return { ...base, state: "unsupported", detail: "Not supported for voice yet" };
    if (p.billing !== "subscription") return { ...base, state: "pay_as_you_go", detail: "Pay-as-you-go profile: voice uses subscription logins only" };
    const login = p.login ?? "claude-max";
    const name = login === "claude-pro" ? "Claude Pro" : "Claude Max";
    if (USED_UP.has(login)) {
      const r = resetsAt();
      if (i < used) until = until === null ? r : Math.min(until, r);
      return { ...base, state: "used_up", detail: `Login ${name} · weekly window used up`, resets_at: r };
    }
    const q = world.status.quotas[p.harness];
    const quota = q?.five_hour && q.weekly ? `5 h window ${Math.round(q.five_hour.used_percent)}%, week ${Math.round(q.weekly.used_percent)}%` : "quota not read yet";
    return { ...base, state: "ready", detail: `Login ${name} · ${quota}` };
  });
  const ready = out.some((s) => s.used && s.state === "ready");
  const unavailable = !voice.enabled ? "Voice is off (Settings → Voice)"
    : used === 0 ? "No voice source: add a Claude subscription profile (Settings → Profiles)"
    : !ready ? (until !== null ? "Every voice login is used up" : "No voice source is ready: check the logins in Settings → Voice")
    : null;
  return { enabled: voice.enabled, language: voice.language, silence_stop_s: voice.silence_stop_s, sources: out, unavailable, resets_at: ready ? null : until };
}

route("GET", "/api/voice", sources);

// ---------------------------------------------------------------- the scripted voice

const PHRASES = ["Also cover the empty cart case.", "Add a test for it before you merge."];
const WORD_MS = 260, PAUSE_MS = 500, CONNECT_MS = 1100;

/** Whether the scripted speaker talks now: the capture's level follows it. */
let speaking = false;

replaceCapture(async (_device, onFrame) => {
  const timer = setInterval(() => {
    const level = speaking ? 0.05 + Math.random() * 0.1 : 0.002 + Math.random() * 0.002;
    onFrame(new ArrayBuffer(3200), level);
  }, 100);
  return { stop: () => { clearInterval(timer); speaking = false; } };
});

onVoiceSocket((socket: FakeSocket) => {
  let run: { id: number; timers: ReturnType<typeof setTimeout>[]; words: string[]; ended: boolean } | null = null;
  const say = (id: number, m: Record<string, unknown>) => socket.deliver(JSON.stringify({ ...m, id }));
  const clear = () => { if (run) { run.timers.forEach(clearTimeout); run.timers = []; } speaking = false; };
  socket.onClosed = clear;
  socket.onSend = (data) => {
    if (typeof data !== "string") return; // Audio frames: the script needs none.
    const m = JSON.parse(data) as { type: string; id?: number; profile?: string };
    if (m.type === "start") {
      clear();
      const id = m.id ?? 0;
      const r = { id, timers: [] as ReturnType<typeof setTimeout>[], words: [] as string[], ended: false };
      run = r;
      const profile = world.config.profiles.find((p) => p.id === m.profile) ?? sources().sources.flatMap((s) => (s.used && s.state === "ready" ? world.config.profiles.filter((p) => p.id === s.profile) : []))[0];
      const at = (ms: number, fn: () => void) => r.timers.push(setTimeout(fn, ms));
      let t = 300;
      at(t, () => { speaking = true; });
      at(CONNECT_MS, () => say(id, { type: "source", name: `Claude Code (${profile?.name ?? "Claude"})` }));
      t = CONNECT_MS + 200;
      for (const phrase of PHRASES) {
        const words = phrase.split(" ");
        words.forEach((_, i) => { at(t, () => { speaking = true; r.words = words.slice(0, i + 1); say(id, { type: "delta", text: r.words.join(" ") }); }); t += WORD_MS; });
        at(t, () => { speaking = false; });
        t += PAUSE_MS;
        at(t, () => { r.words = []; say(id, { type: "done", text: phrase }); });
        t += 300;
      }
      // Then silence: the window's auto-stop counts down and stops, or the owner stops first.
    } else if ((m.type === "stop" || m.type === "cancel") && run && !run.ended) {
      const r = run;
      clear();
      r.ended = true;
      if (m.type === "cancel") return;
      // The last words settle, then the call ends.
      r.timers.push(setTimeout(() => {
        if (r.words.length) say(r.id, { type: "done", text: r.words.join(" ") });
        r.words = [];
        say(r.id, { type: "closed" });
      }, 400));
    }
  };
});
