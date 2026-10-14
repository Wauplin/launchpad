// Phone terminal accessories stay at the bottom of the visible viewport, above the soft keyboard.
// Desktop terminals keep their normal shortcuts and bottom dock.

import { useEffect, useState } from "react";
import { isPhone } from "@/lib/phone";
import { useLayout } from "@/stores/layout";
import { useScopedTerminals } from "@/features/terminals/lib";
import { keyBar, terminalInputs } from "@/features/terminals/keybar";
import { copyText } from "@/lib/clipboard";
import { Icon } from "@/components/Icon";

const keyButtonClass = "btn btn-sm min-w-11 flex-none justify-center font-mono";

function KeyBar() {
  const dock = useLayout((s) => s.dock);
  const terminals = useScopedTerminals();
  const active = terminals.find((t) => t.id === dock.active) ?? terminals[terminals.length - 1];
  const [ctrl, setCtrl] = useState(keyBar.ctrl);
  const [more, setMore] = useState(false);
  const [feedback, setFeedback] = useState("");
  useEffect(() => {
    if (!feedback) return;
    const timer = setTimeout(() => setFeedback(""), 4000);
    return () => clearTimeout(timer);
  }, [feedback]);
  useEffect(() => {
    keyBar.changed = () => setCtrl(keyBar.ctrl);
    return () => { keyBar.ctrl = false; keyBar.changed = () => {}; };
  }, []);
  useEffect(() => { keyBar.ctrl = false; setCtrl(false); setMore(false); setFeedback(""); }, [active?.id]);
  if (!active) return null;
  const target = () => terminalInputs.get(active.id);
  // Preserve the terminal's text selection and soft keyboard while tapping or swiping the bar.
  const keep = (e: React.PointerEvent) => e.preventDefault();
  const type = (data: string) => { const t = target(); t?.input(data, true); t?.focus(); };
  const button = (label: React.ReactNode, description: string, action: () => void, input = true) => (
    <button key={description} type="button" className={keyButtonClass} title={description} aria-label={description} disabled={input && !active.alive} onPointerDown={keep} onClick={action}>{label}</button>
  );
  const copy = () => {
    const t = target();
    const text = t?.getSelection();
    if (!text) { setFeedback("Select terminal text first, or use More → Select all"); return; }
    void copyText(text).then(() => setFeedback("Copied terminal selection")).catch(() => setFeedback("Could not copy terminal selection")).finally(() => { if (active.alive) t?.focus(); });
  };
  return (
    <div className="terminal-keybar relative flex flex-none flex-col gap-1 border-t border-line bg-chrome p-1.5 shadow-[0_-4px_16px_#00000012]" role="toolbar" aria-label="Terminal keys">
      {feedback && <div className="pointer-events-none absolute right-0 bottom-full left-0 border-t border-line bg-chrome px-2 py-1 text-[12px] text-ink-2" role="status">{feedback}</div>}
      <div className="flex gap-1 overflow-x-auto [scrollbar-width:none]">
        {button("Tab", "Tab — complete command", () => type("\t"))}
        {button("Esc", "Escape", () => type("\u001b"))}
        <button type="button" className={`${keyButtonClass} ${ctrl ? "bg-teal-soft! border-teal! text-teal-ink!" : ""}`} disabled={!active.alive} aria-pressed={ctrl} title="Ctrl applies to the next letter you type" onPointerDown={keep} onClick={() => { keyBar.ctrl = !keyBar.ctrl; setCtrl(keyBar.ctrl); target()?.focus(); }}>Ctrl</button>
        {([ ["", "A", "up — previous command"], ["rotate-180", "B", "down — next command"], ["-rotate-90", "D", "left"], ["rotate-90", "C", "right"]] as const).map(([rotation, direction, description]) => button(<Icon name="up" className={rotation} />, `Arrow ${description}`, () => { target()?.arrow(direction); target()?.focus(); }))}
      </div>
      <div className="flex gap-1 overflow-x-auto [scrollbar-width:none]">
        {button("Copy", "Copy selected terminal text", copy, false)}
        {button("Paste", "Paste clipboard into terminal", () => {
          // Capture the emulator before the permission prompt: never paste into a newly selected tab.
          const t = target();
          void (async () => {
            const text = await navigator.clipboard.readText();
            t?.paste(text);
            t?.focus();
          })().catch(() => setFeedback("Clipboard unavailable. Use your keyboard’s Paste action."));
        })}
        {button("Clear", "Clear / redraw — Ctrl+L", () => type("\u000c"))}
        {button("^C", "Ctrl+C — interrupt", () => type("\u0003"))}
        {button("^R", "Ctrl+R — search command history", () => type("\u0012"))}
        <button type="button" className={keyButtonClass} aria-expanded={more} aria-controls="terminal-extra-keys" onPointerDown={keep} onClick={() => setMore(!more)}>More</button>
      </div>
      {more && <div id="terminal-extra-keys" className="flex gap-1 overflow-x-auto [scrollbar-width:none]">
        {button("^A", "Ctrl+A — start of line", () => type("\u0001"))}
        {button("^E", "Ctrl+E — end of line", () => type("\u0005"))}
        {button("^U", "Ctrl+U — delete to start of line", () => type("\u0015"))}
        {button("^W", "Ctrl+W — delete previous word", () => type("\u0017"))}
        {button("^D", "Ctrl+D — end of input / exit", () => type("\u0004"))}
        {button("Select all", "Select all terminal output for copying", () => target()?.selectAll(), false)}
      </div>}
    </div>
  );
}

export function TerminalsView() {
  const setDock = useLayout((s) => s.setDock);
  useEffect(() => { if (!isPhone) setDock({ open: true }); }, [setDock]);
  if (isPhone) return <KeyBar />;
  return <div className="grid h-full place-items-center p-8 text-center text-ink-2">Terminals open in the dock at the bottom of the window.</div>;
}
