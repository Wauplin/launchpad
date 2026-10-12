// One integrated terminal (xterm.js, DOM renderer: WebGL is unreliable in WebKitGTK). It has its own
// WebSocket: the daemon sends the buffered output, then live output; keystrokes go back as binary frames.

import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { copyText } from "@/lib/clipboard";
import { toastError } from "@/components/Toast";

const theme = { background: "#FFFFFF", foreground: "#17202B", cursor: "#16766C", cursorAccent: "#FFFFFF", selectionBackground: "#CDE9E4", black: "#17202B", brightBlack: "#637184", red: "#B42318", brightRed: "#B42318", green: "#1A7F37", brightGreen: "#1A7F37", yellow: "#8A5D00", brightYellow: "#8A5D00", blue: "#1F63B0", brightBlue: "#1F63B0", magenta: "#8250DF", brightMagenta: "#8250DF", cyan: "#0D7266", brightCyan: "#0D7266", white: "#4B5767", brightWhite: "#17202B" };

/**
 * The interface zoom is CSS `zoom` on `<html>`, which breaks xterm.js mouse coordinates. The container
 * cancels it (`zoom: 1/scale`) and the font is scaled instead, so xterm.js measures real pixels.
 */
export function TerminalView({ id, scale }: { id: string; scale: number }) {
  const host = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!host.current) return;
    let disposed = false;
    let socket: WebSocket | null = null;
    let retry = 0;
    const t = new Terminal({ theme, fontFamily: '"DejaVu Sans Mono", "Menlo", "Consolas", monospace', fontSize: Math.round(12 * scale), scrollback: 5000, allowProposedApi: false });
    const fit = new FitAddon();
    t.loadAddon(fit);
    t.open(host.current);
    const mac = /Mac/.test(navigator.platform);
    t.attachCustomKeyEventHandler((event) => {
      const key = event.key.toLowerCase();
      const command = mac && event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey;
      const control = event.ctrlKey && !event.metaKey && !event.altKey;
      const copy = key === "c" && (command || (control && event.shiftKey));
      const paste = key === "v" && (command || control);
      const clear = key === "l" && command;
      if (!copy && !paste && !clear) return true;
      // Native Ctrl+V/Cmd+V fires xterm's paste event, even in webviews without the async
      // clipboard API. Bypass xterm's key translation without cancelling that default.
      if (paste && !event.shiftKey) return false;
      event.preventDefault();
      if (event.type !== "keydown" || event.repeat) return false;
      if (copy) {
        const text = t.getSelection();
        if (text && !document.execCommand("copy")) {
          void copyText(text).catch(toastError).finally(() => { if (!disposed) t.focus(); });
        }
      } else if (paste) {
        // Ctrl+Shift+V isn't a native paste binding in every webview. Try its native
        // paste command first, then the browser clipboard API. t.paste preserves bracketed paste.
        if (!document.execCommand("paste")) {
          void (async () => {
            const text = await navigator.clipboard.readText();
            if (!disposed) t.paste(text);
          })().catch(toastError);
        }
      } else {
        // Cmd+L is the macOS alias; Ctrl+L goes through unchanged to readline/the TUI.
        t.input("\u000c", true);
      }
      return false;
    });
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
    return () => { disposed = true; observer.disconnect(); input.dispose(); binary.dispose(); resized.dispose(); socket?.close(); t.dispose(); };
  }, [id, scale]);

  return (
    <div className="min-h-0 flex-1 overflow-hidden bg-white px-2 py-1" style={{ zoom: 1 / scale }}>
      <div ref={host} className="h-full w-full" />
    </div>
  );
}
