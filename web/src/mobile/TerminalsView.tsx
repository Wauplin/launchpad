// Terminals page. On the phone it shows a key bar (Esc, Tab, sticky Ctrl, arrows, Paste) above the full-page
// terminal dock (MobileShell), since soft keyboards lack those keys. The desktop keeps terminals in the bottom
// dock, so this page only opens it.

import { useEffect, useState } from "react";
import { isPhone } from "@/lib/phone";
import { useLayout } from "@/stores/layout";
import { useScopedTerminals } from "@/features/terminals/lib";
import { keyBar, terminalInputs } from "@/features/terminals/keybar";
import { toastError } from "@/components/Toast";

const KEYS: [string, string][] = [["Esc", "\u001b"], ["Tab", "\t"], ["↑", "\u001b[A"], ["↓", "\u001b[B"], ["←", "\u001b[D"], ["→", "\u001b[C"]];

function KeyBar() {
  const dock = useLayout((s) => s.dock);
  const terminals = useScopedTerminals();
  const active = terminals.find((t) => t.id === dock.active) ?? terminals[terminals.length - 1];
  const [ctrl, setCtrl] = useState(keyBar.ctrl);
  useEffect(() => { keyBar.changed = () => setCtrl(keyBar.ctrl); return () => { keyBar.changed = () => {}; }; }, []);
  if (!active?.alive) return null;
  const target = () => terminalInputs.get(active.id);
  // Keep the soft keyboard and the terminal focus: buttons do not take focus on press.
  const keep = (e: React.PointerEvent) => e.preventDefault();
  const type = (data: string) => { const t = target(); t?.input(data, true); t?.focus(); };
  return (
    <div className="flex flex-none gap-1 overflow-x-auto border-t border-line bg-chrome px-1.5 py-1" role="toolbar" aria-label="Terminal keys">
      {KEYS.slice(0, 2).map(([label, data]) => <button key={label} className="btn btn-sm flex-none" onPointerDown={keep} onClick={() => type(data)}>{label}</button>)}
      <button className={`btn btn-sm flex-none ${ctrl ? "bg-teal-soft! border-teal! text-teal-ink!" : ""}`} aria-pressed={ctrl} title="Ctrl applies to the next letter you type" onPointerDown={keep} onClick={() => { keyBar.ctrl = !keyBar.ctrl; setCtrl(keyBar.ctrl); target()?.focus(); }}>Ctrl</button>
      {KEYS.slice(2).map(([label, data]) => <button key={label} className="btn btn-sm flex-none" aria-label={`Arrow ${label}`} onPointerDown={keep} onClick={() => type(data)}>{label}</button>)}
      <button className="btn btn-sm flex-none" onPointerDown={keep} onClick={() => void navigator.clipboard.readText().then((text) => { const t = target(); t?.paste(text); t?.focus(); }).catch(toastError)}>Paste</button>
    </div>
  );
}

export function TerminalsView() {
  const setDock = useLayout((s) => s.setDock);
  useEffect(() => { if (!isPhone) setDock({ open: true }); }, [setDock]);
  if (isPhone) return <KeyBar />;
  return <div className="grid h-full place-items-center p-8 text-center text-ink-2">Terminals open in the dock at the bottom of the window.</div>;
}
