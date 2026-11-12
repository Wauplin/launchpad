// The mic of a message box, just left of Send: a tap starts and stops dictation; on a touch screen a long press
// (≥ 350 ms) talks until release; hovering or focusing it warms the connection up. While recording it is a red stop
// button with the auto-stop countdown ring. Disabled with the reason in its tooltip.

import { useRef } from "react";
import { Icon } from "./Icon";
import { useVoice } from "@/lib/voice/dictation";
import type { Dictation } from "@/lib/voice/useDictation";

const LONG_PRESS_MS = 350;

export function VoiceButton({ dictation: d, className = "", large }: { dictation: Dictation; className?: string; large?: boolean }) {
  const countdown = useVoice((s) => (s.owner === d.id ? s.countdown : null));
  const press = useRef<{ timer: ReturnType<typeof setTimeout> | null; held: boolean }>({ timer: null, held: false });
  if (!d.shown) return null;
  const label = d.active ? (d.phase === "finishing" ? "Finishing…" : "Stop dictation (Esc)") : d.reason ?? "Dictate (Ctrl+Space; long press on a phone to hold)";
  const ring = countdown !== null && <svg viewBox="0 0 20 20" className="pointer-events-none absolute inset-0 size-full -rotate-90" aria-hidden>
    <circle cx="10" cy="10" r="9" fill="none" stroke="currentColor" strokeWidth="1.5" strokeDasharray={`${(1 - countdown) * 56.5} 56.5`} />
  </svg>;
  return (
    <button type="button" title={label} aria-label={d.active ? "Stop dictation" : "Dictate"} aria-pressed={d.active}
      disabled={!d.active && !!d.reason}
      className={`${large ? "btn relative h-[52px] flex-1 justify-center text-[15px]" : "iconbtn relative"} touch-manipulation select-none ${d.active ? "border-danger! bg-danger-ink text-paper hover:bg-danger! hover:text-paper!" : ""} ${className}`}
      onPointerEnter={d.warm} onFocus={d.warm}
      onContextMenu={(e) => e.preventDefault()}
      onPointerDown={(e) => {
        if (e.pointerType !== "touch" || d.active) return;
        press.current.held = false;
        press.current.timer = setTimeout(() => { press.current.held = true; d.start(true); }, LONG_PRESS_MS);
      }}
      onPointerUp={() => {
        if (press.current.timer) clearTimeout(press.current.timer);
        press.current.timer = null;
        if (press.current.held) void d.stop();
      }}
      onPointerCancel={() => { if (press.current.timer) clearTimeout(press.current.timer); press.current.timer = null; }}
      onClick={() => {
        // The click that ends a long press is not a tap.
        if (press.current.held) { press.current.held = false; return; }
        d.toggle();
      }}>
      <Icon name={d.active ? "stop" : "mic"} size={14} />
      {large && (d.active ? "Stop" : "Dictate")}
      {ring}
    </button>
  );
}
