// One integrated terminal (xterm.js, DOM renderer: WebGL is unreliable in WebKitGTK). It has its own
// WebSocket: the daemon sends the buffered output, then live output; keystrokes go back as binary frames.

import { forwardRef, useEffect, useImperativeHandle, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { copyText } from "@/lib/clipboard";
import { toast, toastError } from "@/components/Toast";

export type TerminalHandle = { copy: () => void; paste: () => void; clear: () => void; focus: () => void };

const theme = { background: "#FFFFFF", foreground: "#17202B", cursor: "#16766C", cursorAccent: "#FFFFFF", selectionBackground: "#CDE9E4", black: "#17202B", brightBlack: "#637184", red: "#B42318", brightRed: "#B42318", green: "#1A7F37", brightGreen: "#1A7F37", yellow: "#8A5D00", brightYellow: "#8A5D00", blue: "#1F63B0", brightBlue: "#1F63B0", magenta: "#8250DF", brightMagenta: "#8250DF", cyan: "#0D7266", brightCyan: "#0D7266", white: "#4B5767", brightWhite: "#17202B" };

/**
 * The interface zoom is CSS `zoom` on `<html>`, which breaks xterm.js mouse coordinates. The container
 * cancels it (`zoom: 1/scale`) and the font is scaled instead, so xterm.js measures real pixels.
 */
export const TerminalView = forwardRef<TerminalHandle, { id: string; scale: number }>(function TerminalView({ id, scale }, ref) {
  const host = useRef<HTMLDivElement>(null);
  const term = useRef<Terminal | null>(null);

  useImperativeHandle(ref, () => ({
    copy: () => {
      const text = term.current?.getSelection() ?? "";
      if (!text) return toast("Select text in the terminal first");
      void copyText(text).then(() => toast("Copied")).catch(toastError);
    },
    paste: () => void navigator.clipboard.readText().then((text) => { term.current?.paste(text); term.current?.focus(); }).catch(toastError),
    clear: () => { term.current?.clear(); term.current?.focus(); },
    focus: () => term.current?.focus(),
  }), []);

  useEffect(() => {
    if (!host.current) return;
    let disposed = false;
    let socket: WebSocket | null = null;
    let retry = 0;
    const t = new Terminal({ theme, fontFamily: '"DejaVu Sans Mono", "Menlo", "Consolas", monospace', fontSize: Math.round(12 * scale), scrollback: 5000, allowProposedApi: false });
    const fit = new FitAddon();
    t.loadAddon(fit);
    t.open(host.current);
    term.current = t;
    const encoder = new TextEncoder();
    const send = (data: Uint8Array<ArrayBuffer>) => { if (socket?.readyState === WebSocket.OPEN) socket.send(data); };
    const input = t.onData((d) => send(encoder.encode(d)));
    const binary = t.onBinary((d) => send(Uint8Array.from(d, (c) => c.charCodeAt(0) & 0xff)));
    const sendSize = () => { if (socket?.readyState === WebSocket.OPEN) socket.send(JSON.stringify({ type: "resize", rows: t.rows, cols: t.cols })); };
    const resized = t.onResize(sendSize);
    const refit = () => { if (!disposed && host.current?.clientWidth && host.current.clientHeight) fit.fit(); };
    const connect = () => {
      const ws = new WebSocket(`${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/api/terminals/${id}/ws`);
      ws.binaryType = "arraybuffer";
      socket = ws;
      ws.onopen = () => { retry = 0; refit(); sendSize(); };
      ws.onmessage = (msg) => {
        if (typeof msg.data !== "string") { t.write(new Uint8Array(msg.data as ArrayBuffer)); return; }
        // `exit` also arrives on the bus, which updates the tab.
        if ((JSON.parse(msg.data) as { type: string }).type === "reset") t.reset();
      };
      // The daemon may have restarted; the list refresh then removes this tab.
      ws.onclose = () => { if (!disposed && retry < 5) setTimeout(connect, 500 * 2 ** retry++); };
    };
    connect();
    const observer = new ResizeObserver(refit);
    observer.observe(host.current);
    t.focus();
    return () => { disposed = true; observer.disconnect(); input.dispose(); binary.dispose(); resized.dispose(); socket?.close(); t.dispose(); term.current = null; };
  }, [id, scale]);

  return (
    <div className="min-h-0 flex-1 overflow-hidden bg-white px-2 py-1" style={{ zoom: 1 / scale }}>
      <div ref={host} className="h-full w-full" />
    </div>
  );
});
