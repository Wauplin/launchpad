import { useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { api } from "@/lib/api";
import { onConnectionChange, onEvent, type BusEvent } from "@/lib/ws";
import { copyText } from "@/lib/clipboard";
import { toastError } from "@/components/Toast";

type Snapshot = { running: boolean; output: string; truncated: boolean; end: number };
type Chunk = { data: string; start: number; end: number };
const bytes = (data: string) => Uint8Array.from(atob(data), (c) => c.charCodeAt(0));
const encode = (data: string) => btoa(Array.from(new TextEncoder().encode(data), (b) => String.fromCharCode(b)).join(""));

export function EmbeddedTerminal({ sessionId, onReturn }: { sessionId: string; onReturn: () => void }) {
  const host = useRef<HTMLDivElement>(null);
  const terminal = useRef<Terminal | null>(null);
  const send = useRef<(data: string) => void>(() => {});
  const [running, setRunning] = useState(false);
  const [truncated, setTruncated] = useState(false);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    if (!host.current) return;
    let disposed = false;
    let syncing = true;
    let version = 0;
    let end = 0;
    let alive = false;
    let pending: BusEvent[] = [];
    let inputQueue = Promise.resolve();
    const term = new Terminal({
      theme: { background: "#FFFFFF", foreground: "#17202B", cursor: "#16766C", selectionBackground: "#CDE9E4", black: "#17202B", brightBlack: "#637184", red: "#B42318", brightRed: "#B42318", green: "#1A7F37", brightGreen: "#1A7F37", yellow: "#8A5D00", brightYellow: "#8A5D00", blue: "#1F63B0", brightBlue: "#1F63B0", magenta: "#8250DF", brightMagenta: "#8250DF", cyan: "#0D7266", brightCyan: "#0D7266", white: "#4B5767", brightWhite: "#17202B" },
      fontFamily: '"DejaVu Sans Mono", monospace', fontSize: 12, scrollback: 5000, allowProposedApi: false,
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(host.current);
    terminal.current = term;
    send.current = (data) => {
      if (!alive) return;
      inputQueue = inputQueue.then(() => api.post(`/api/sessions/${sessionId}/terminal/input`, { data: encode(data) })).then(() => {}).catch((e) => { if (!disposed) setError(String(e)); });
    };
    const input = term.onData((data) => send.current(data));
    const resize = () => {
      if (disposed || !host.current?.clientWidth || !host.current.clientHeight) return;
      fit.fit();
      void api.post(`/api/sessions/${sessionId}/terminal/resize`, { rows: term.rows, cols: term.cols }).catch(() => {});
    };
    const apply = (ev: BusEvent) => {
      if (ev.type === "exit") { alive = false; setRunning(false); return; }
      if (ev.type !== "output") return;
      const chunk = ev.payload as Chunk;
      if (chunk.end <= end) return;
      term.write(bytes(chunk.data));
      end = chunk.end;
    };
    const off = onEvent((ev) => {
      if (ev.topic !== `terminal/${sessionId}`) return;
      if (ev.type === "resync") { void sync(); return; }
      if (syncing) pending.push(ev); else apply(ev);
    });
    const sync = async () => {
      syncing = true;
      const current = ++version;
      try {
        const snapshot = await api.get<Snapshot>(`/api/sessions/${sessionId}/terminal/snapshot`);
        if (disposed || current !== version) return;
        alive = snapshot.running;
        term.reset();
        term.write(bytes(snapshot.output));
        end = snapshot.end;
        setRunning(snapshot.running);
        setTruncated(snapshot.truncated);
        setError("");
        for (const ev of pending) apply(ev);
        pending = [];
        syncing = false;
        resize();
      } catch (e) { if (!disposed) setError(String(e)); }
    };
    const connection = onConnectionChange((connected) => { if (connected) void sync(); else { alive = false; setRunning(false); } });
    void sync();
    const observer = new ResizeObserver(resize);
    observer.observe(host.current);
    return () => { disposed = true; off(); connection(); observer.disconnect(); input.dispose(); term.dispose(); terminal.current = null; send.current = () => {}; };
  }, [sessionId]);
  return <div className="flex min-h-0 flex-1 flex-col">
    <div className="flex flex-wrap items-center gap-1 border-b border-line px-2 py-1 text-xs">
      <b className="mr-1">Terminal</b><span className="mr-auto text-ink-3">{running ? "Full harness TUI" : "TUI stopped"}</span>
      <button className="btn btn-sm" onClick={() => terminal.current?.focus()}>Focus</button>
      <button className="btn btn-sm" onClick={() => void copyText(terminal.current?.getSelection() ?? "").catch(toastError)}>Copy selection</button>
      <button className="btn btn-sm" disabled={!running} onClick={() => void navigator.clipboard.readText().then((text) => send.current(text)).catch(toastError)}>Paste</button>
      {[["Enter", "\r"], ["Escape", "\x1b"], ["↑", "\x1b[A"], ["↓", "\x1b[B"], ["←", "\x1b[D"], ["→", "\x1b[C"], ["Interrupt", "\x03"]].map(([label, data]) => <button key={label} className="btn btn-sm" disabled={!running} onClick={() => send.current(data)}>{label}</button>)}
      <button className="btn btn-sm" disabled={busy} onClick={async () => {
        setBusy(true);
        try {
          await api.post(`/api/sessions/${sessionId}/terminal/stop`);
          await api.post(`/api/sessions/${sessionId}/resume`);
          onReturn();
        } catch (e) { toastError(e); } finally { setBusy(false); }
      }}>Stop TUI and re-attach</button>
    </div>
    {truncated && <div className="px-2 py-1 text-xs text-ink-3">Earlier terminal output was trimmed (1 MB retained).</div>}
    {error && <div className="px-2 py-1 text-xs text-attn">{error}</div>}
    <div ref={host} className="min-h-0 flex-1 overflow-hidden bg-white p-2" />
  </div>;
}
