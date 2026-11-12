// The caption strip over a message box while it dictates: state and source, a level meter, the timer and the words
// of the current phrase. After a failover one quiet line says which login took over; an error is one line too.

import { useEffect, useState } from "react";
import { Icon } from "./Icon";
import { IconButton } from "./ui";
import { clearVoiceError, useVoice } from "@/lib/voice/dictation";
import type { Dictation } from "@/lib/voice/useDictation";

const clock = (ms: number) => { const s = Math.max(0, Math.floor(ms / 1000)); return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`; };

function Timer({ since }: { since: number }) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => { const t = setInterval(() => setNow(Date.now()), 1000); return () => clearInterval(t); }, []);
  return <span className="flex-none tabular-nums text-ink-3">{clock(now - since)}</span>;
}

function Meter() {
  const level = useVoice((s) => s.level);
  return <span className="flex h-3 flex-none items-end gap-px" aria-hidden>
    {[0.15, 0.35, 0.6, 0.85].map((step, i) => <span key={i} className={`w-[3px] rounded-full ${level >= step ? "bg-danger" : "bg-meter"}`} style={{ height: `${4 + i * 2}px` }} />)}
  </span>;
}

/** `inline`: no interim words (the top-bar field shows them in the field itself). */
export function VoiceCaption({ dictation: d, className = "mb-1.5", inline }: { dictation: Dictation; className?: string; inline?: boolean }) {
  const mine = useVoice((s) => s.owner === d.id);
  const phase = useVoice((s) => s.phase);
  const source = useVoice((s) => s.source);
  const interim = useVoice((s) => s.interim);
  const note = useVoice((s) => s.note);
  const error = useVoice((s) => s.error);
  const startedAt = useVoice((s) => s.startedAt);
  if (!mine) return null;
  if (phase === "idle") {
    if (error) return <div role="alert" className={`flex items-center gap-1.5 text-[12px] text-danger-ink ${className}`}><Icon name="alert" size={12} className="flex-none" /><span className={`min-w-0 flex-1 ${inline ? "truncate" : ""}`} title={inline ? error : undefined}>{error}</span><IconButton icon="x" label="Dismiss" className="-my-1.5 flex-none" onClick={() => clearVoiceError(d.id)} /></div>;
    return null;
  }
  const state = phase === "starting" ? "Connecting…" : phase === "finishing" ? "Finishing…" : "Listening";
  return (
    <div role="status" aria-live="polite" className={`grid gap-0.5 text-[12px] ${className}`}>
      {note && <div className="flex items-center gap-1.5 text-ink-3"><Icon name="refresh" size={12} className="flex-none" /><span className="min-w-0 flex-1 truncate" title={note}>{note}</span></div>}
      <div className="flex min-w-0 items-center gap-1.5">
        {phase === "finishing" ? <span className="lp-spin flex-none" aria-hidden /> : <Meter />}
        <span className="min-w-0 truncate font-semibold text-ink-2">{state}{source && phase !== "starting" ? ` · ${source}` : ""}</span>
        <Timer since={startedAt} />
      </div>
      {!inline && interim && <div className="line-clamp-2 italic text-ink-3">{interim}</div>}
    </div>
  );
}
