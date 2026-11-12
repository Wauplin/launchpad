// The microphone: an AudioWorklet turns it into 16 kHz mono 16-bit PCM frames of 100 ms, with their level.
// Recording starts at the click, before any network: frames wait in the voice socket's queue until Home's call is up.

const WORKLET = `
class Pcm16k extends AudioWorkletProcessor {
  constructor() { super(); this.step = sampleRate / 16000; this.pos = 0; this.acc = 0; this.n = 0; this.last = 0; this.out = new Int16Array(1600); this.len = 0; this.sq = 0; }
  process(inputs) {
    const input = inputs[0] && inputs[0][0];
    if (!input) return true;
    for (let i = 0; i < input.length; i++) {
      this.acc += input[i]; this.n++; this.pos += 1;
      // Each output sample averages the input samples of its period; below 16 kHz (a headset), samples repeat.
      while (this.pos >= this.step) {
        this.pos -= this.step;
        const v = this.n ? Math.max(-1, Math.min(1, this.acc / this.n)) : this.last;
        this.last = v; this.acc = 0; this.n = 0;
        this.sq += v * v;
        this.out[this.len++] = v < 0 ? v * 0x8000 : v * 0x7fff;
        if (this.len === this.out.length) {
          this.port.postMessage({ pcm: this.out.buffer.slice(0), level: Math.sqrt(this.sq / this.len) });
          this.len = 0; this.sq = 0;
        }
      }
    }
    return true;
  }
}
registerProcessor("pcm16k", Pcm16k);
`;

export type Capture = { stop: () => void };

/** Why the microphone could not start, in words for the caption. */
export function micError(e: unknown): string {
  const name = e instanceof DOMException ? e.name : "";
  if (name === "NotAllowedError" || name === "SecurityError") return "Microphone blocked for this site. Allow it in the browser's site settings (the icon left of the address), then try again.";
  if (name === "NotFoundError" || name === "OverconstrainedError") return "No microphone found. Plug one in or pick another in Settings → Voice.";
  if (name === "NotReadableError") return "The microphone is used by another app.";
  return `The microphone did not start${e instanceof Error && e.message ? `: ${e.message}` : ""}.`;
}

export type CaptureSource = (deviceId: string | null, onFrame: (pcm: ArrayBuffer, level: number) => void) => Promise<Capture>;
let replaced: CaptureSource | null = null;
/** The demo plays a scripted voice: its frames come from here instead of a microphone (no permission prompt). */
export function replaceCapture(source: CaptureSource) { replaced = source; }

export async function startCapture(deviceId: string | null, onFrame: (pcm: ArrayBuffer, level: number) => void): Promise<Capture> {
  if (replaced) return replaced(deviceId, onFrame);
  const constraints = (id: string | null): MediaStreamConstraints => ({ audio: { ...(id ? { deviceId: { exact: id } } : {}), channelCount: 1, echoCancellation: true, noiseSuppression: true, autoGainControl: true } });
  // Created before the first await, while the click still counts as a user gesture (Safari keeps it suspended otherwise).
  const ctx = new AudioContext();
  let stream: MediaStream;
  try {
    try { stream = await navigator.mediaDevices.getUserMedia(constraints(deviceId)); }
    catch (e) {
      // The chosen microphone is gone: the default one.
      if (!deviceId || !(e instanceof DOMException) || (e.name !== "OverconstrainedError" && e.name !== "NotFoundError")) throw e;
      stream = await navigator.mediaDevices.getUserMedia(constraints(null));
    }
  } catch (e) {
    void ctx.close();
    throw e;
  }
  try {
    const url = URL.createObjectURL(new Blob([WORKLET], { type: "text/javascript" }));
    try { await ctx.audioWorklet.addModule(url); } finally { URL.revokeObjectURL(url); }
    const source = ctx.createMediaStreamSource(stream);
    const node = new AudioWorkletNode(ctx, "pcm16k");
    node.port.onmessage = (ev: MessageEvent<{ pcm: ArrayBuffer; level: number }>) => onFrame(ev.data.pcm, ev.data.level);
    source.connect(node);
    // The worklet runs only when connected to the output; it writes nothing to it.
    const mute = ctx.createGain();
    mute.gain.value = 0;
    node.connect(mute).connect(ctx.destination);
    if (ctx.state === "suspended") await ctx.resume();
    return { stop: () => { node.port.onmessage = null; stream.getTracks().forEach((t) => t.stop()); void ctx.close(); } };
  } catch (e) {
    stream.getTracks().forEach((t) => t.stop());
    void ctx.close();
    throw e;
  }
}

/** The microphones of this device, once the site may use one (labels are empty before). */
export async function microphones(): Promise<MediaDeviceInfo[]> {
  if (!navigator.mediaDevices?.enumerateDevices) return [];
  return (await navigator.mediaDevices.enumerateDevices()).filter((d) => d.kind === "audioinput");
}
