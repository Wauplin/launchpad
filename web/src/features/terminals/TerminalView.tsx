// One integrated terminal (xterm.js, DOM renderer: WebGL is unreliable in WebKitGTK). It has its own
// WebSocket: the daemon sends the buffered output, then live output; keystrokes go back as binary frames.

import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { copyText } from "@/lib/clipboard";
import { toastError } from "@/components/Toast";
import { terminalOutput } from "./output";

import { useTheme } from "@/stores/theme";

const lightTheme = { background: "#FFFFFF", foreground: "#17202B", cursor: "#16766C", cursorAccent: "#FFFFFF", selectionBackground: "#CDE9E4", black: "#17202B", brightBlack: "#637184", red: "#B42318", brightRed: "#B42318", green: "#1A7F37", brightGreen: "#1A7F37", yellow: "#8A5D00", brightYellow: "#8A5D00", blue: "#1F63B0", brightBlue: "#1F63B0", magenta: "#8250DF", brightMagenta: "#8250DF", cyan: "#0D7266", brightCyan: "#0D7266", white: "#4B5767", brightWhite: "#17202B" };

const darkTheme = { background: "#0D1117", foreground: "#E6EDF3", cursor: "#79D7C6", cursorAccent: "#0D1117", selectionBackground: "#23364D", black: "#484F58", brightBlack: "#98A6B7", red: "#FF939A", brightRed: "#FFA8AD", green: "#70D68B", brightGreen: "#8DE5A2", yellow: "#EAC76A", brightYellow: "#F4DB91", blue: "#83BAFF", brightBlue: "#A5CEFF", magenta: "#C3A0FF", brightMagenta: "#D5BAFF", cyan: "#79D7C6", brightCyan: "#9BE6D9", white: "#B6C2CF", brightWhite: "#E6EDF3" };

/**
 * The interface zoom is CSS `zoom` on `<html>`, which breaks xterm.js mouse coordinates. The container
 * cancels it (`zoom: 1/scale`) and the font is scaled instead, so xterm.js measures real pixels.
 */
export function TerminalView({ id, scale, visible = true }: { id: string; scale: number; visible?: boolean }) {
  const theme = useTheme((s) => s.theme);
  const host = useRef<HTMLDivElement>(null);
  const instance = useRef<{ terminal: Terminal; refit: () => void } | null>(null);
  const currentScale = useRef(scale);
  currentScale.current = scale;
  const currentVisible = useRef(visible);
  currentVisible.current = visible;

  useEffect(() => {
    if (!host.current) return;
    let disposed = false;
    let socket: WebSocket | null = null;
    let retry = 0;
    let retryTimer: ReturnType<typeof setTimeout> | undefined;
    const t = new Terminal({ theme: useTheme.getState().theme === "dark" ? darkTheme : lightTheme, fontFamily: '"DejaVu Sans Mono", "Menlo", "Consolas", monospace', fontSize: Math.round(12 * currentScale.current), scrollback: 5000, allowProposedApi: false });
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
    const sendSize = () => { if (currentVisible.current && !output?.replaying && socket?.readyState === WebSocket.OPEN) socket.send(JSON.stringify({ type: "resize", rows: t.rows, cols: t.cols })); };
    const resized = t.onResize(sendSize);
    const refit = () => { if (!disposed && !output?.replaying && host.current?.clientWidth && host.current.clientHeight) fit.fit(); };
    let output: ReturnType<typeof terminalOutput> | null = null;
    instance.current = { terminal: t, refit };
    const connect = () => {
      if (disposed) return;
      const ws = new WebSocket(`${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/api/terminals/${id}/ws`);
      ws.binaryType = "arraybuffer";
      socket = ws;
      const stream = terminalOutput(t, (epoch, bytes) => {
        if (!disposed && ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify({ type: "ack", epoch, bytes }));
      }, () => { if (!disposed && socket === ws) { refit(); sendSize(); } });
      output = stream;
      ws.onopen = () => { retry = 0; };
      ws.onmessage = (msg) => {
        if (disposed || socket !== ws) return;
        if (typeof msg.data !== "string") { stream.data(new Uint8Array(msg.data as ArrayBuffer)); return; }
        // `exit` also arrives on the bus, which updates the tab.
        const control = JSON.parse(msg.data) as { type: string; epoch: number; rows: number; cols: number };
        if (control.type === "reset") stream.reset(control.epoch, control.rows, control.cols);
        if (control.type === "ready") stream.ready();
      };
      // The daemon may have restarted; the list refresh then removes this tab.
      ws.onclose = () => { if (!disposed && retry < 5) retryTimer = setTimeout(connect, 500 * 2 ** retry++); };
    };
    connect();
    const observer = new ResizeObserver(refit);
    observer.observe(host.current);
    return () => { disposed = true; clearTimeout(retryTimer); instance.current = null; observer.disconnect(); input.dispose(); binary.dispose(); resized.dispose(); socket?.close(); t.dispose(); };
  }, [id]);

  useEffect(() => {
    if (instance.current) instance.current.terminal.options.theme = theme === "dark" ? darkTheme : lightTheme;
  }, [id, theme]);

  useEffect(() => {
    const view = instance.current;
    if (!view) return;
    view.terminal.options.fontSize = Math.round(12 * scale);
    if (visible) { view.refit(); view.terminal.focus(); }
  }, [id, scale, visible]);

  return (
    <div className="min-h-0 flex-1 overflow-hidden bg-paper px-2 py-1" style={{ zoom: 1 / scale, display: visible ? undefined : "none" }}>
      <div ref={host} className="h-full w-full" />
    </div>
  );
}
