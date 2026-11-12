// This device's voice choices, in browser storage like theme and zoom: the microphone (`launchpad.voice.mic`, a
// device ID; none: the default one) and the start and stop sounds (`launchpad.voice.sounds`, on unless "off").
// The demo keeps them in memory.

import { create } from "zustand";
import { isDemo } from "../../demo/mode";
import { storageAllowed } from "../clientStorage";

const MIC = "launchpad.voice.mic", SOUNDS = "launchpad.voice.sounds";
const read = (key: string) => { if (isDemo) return null; try { return localStorage.getItem(key); } catch { return null; } };
const write = (key: string, value: string | null) => {
  if (isDemo || !storageAllowed()) return;
  try { if (value === null) localStorage.removeItem(key); else localStorage.setItem(key, value); } catch { /* This window only. */ }
};

export const useVoiceDevice = create<{ mic: string | null; sounds: boolean; setMic: (id: string | null) => void; setSounds: (on: boolean) => void }>((set) => ({
  mic: read(MIC),
  sounds: read(SOUNDS) !== "off",
  setMic: (mic) => { write(MIC, mic); set({ mic }); },
  setSounds: (sounds) => { write(SOUNDS, sounds ? null : "off"); set({ sounds }); },
}));

let audio: AudioContext | null = null;
/** A soft tick: rising at the start, falling at the stop. */
export function tick(kind: "start" | "stop") {
  if (!useVoiceDevice.getState().sounds) return;
  try {
    audio ??= new AudioContext();
    const t = audio.currentTime, osc = audio.createOscillator(), gain = audio.createGain();
    osc.type = "sine";
    osc.frequency.setValueAtTime(kind === "start" ? 660 : 880, t);
    osc.frequency.linearRampToValueAtTime(kind === "start" ? 880 : 620, t + 0.09);
    gain.gain.setValueAtTime(0.0001, t);
    gain.gain.exponentialRampToValueAtTime(0.08, t + 0.015);
    gain.gain.exponentialRampToValueAtTime(0.0001, t + 0.12);
    osc.connect(gain).connect(audio.destination);
    osc.start(t);
    osc.stop(t + 0.13);
  } catch { /* No sound on this device. */ }
}
