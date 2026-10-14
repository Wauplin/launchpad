// One integrated terminal (xterm.js, DOM renderer: WebGL is unreliable in WebKitGTK). It has its own
// WebSocket: the daemon sends the buffered output, then live output; keystrokes go back as binary frames.
// The view counts the output bytes it handed to the emulator (absolute offsets from the daemon); after
// a disconnect it reconnects with `?offset=` and receives only what it missed when the daemon still has
// it. Reconnection never gives up (capped backoff) and happens at once when the page becomes visible
// again or the network comes back.
//
// Several viewers (windows, devices) share one PTY size. The daemon says which viewer owns it: the one
// that typed last, else the first one. The owner fits its container; the others render at the PTY size,
// scrollable, with a Use this size button. Every viewer reports the size it would fit.

import { useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { keyBar, terminalInputs } from "./keybar";
import { copyText } from "@/lib/clipboard";
import { toastError } from "@/components/Toast";
import { terminalOutput } from "./output";

import { useTheme } from "@/stores/theme";

const lightTheme = { background: "#FFFFFF", foreground: "#17202B", cursor: "#16766C", cursorAccent: "#FFFFFF", selectionBackground: "#CDE9E4", black: "#17202B", brightBlack: "#637184", red: "#B42318", brightRed: "#B42318", green: "#1A7F37", brightGreen: "#1A7F37", yellow: "#8A5D00", brightYellow: "#8A5D00", blue: "#1F63B0", brightBlue: "#1F63B0", magenta: "#8250DF", brightMagenta: "#8250DF", cyan: "#0D7266", brightCyan: "#0D7266", white: "#4B5767", brightWhite: "#17202B" };

const darkTheme = { background: "#0D1117", foreground: "#E6EDF3", cursor: "#79D7C6", cursorAccent: "#0D1117", selectionBackground: "#23364D", black: "#484F58", brightBlack: "#98A6B7", red: "#FF939A", brightRed: "#FFA8AD", green: "#70D68B", brightGreen: "#8DE5A2", yellow: "#EAC76A", brightYellow: "#F4DB91", blue: "#83BAFF", brightBlue: "#A5CEFF", magenta: "#C3A0FF", brightMagenta: "#D5BAFF", cyan: "#79D7C6", brightCyan: "#9BE6D9", white: "#B6C2CF", brightWhite: "#E6EDF3" };

type Owner = "self" | "other" | "none";
type Control = { type: string; epoch: number; offset: number; rows: number; cols: number; owner: Owner };

/** No message for this long (the daemon pings every 25 s): the connection is dead, replace it. */
const STALE_MS = 60_000;
const MAX_RETRY_MS = 10_000;

/**
 * The interface zoom is CSS `zoom` on `<html>`, which breaks xterm.js mouse coordinates. The container
 * cancels it (`zoom: 1/scale`) and the font is scaled instead, so xterm.js measures real pixels.
 */
export function TerminalView({ id, scale, visible = true }: { id: string; scale: number; visible?: boolean }) {
  const theme = useTheme((s) => s.theme);
  const host = useRef<HTMLDivElement>(null);
  const instance = useRef<{ terminal: Terminal; refit: () => void; claim: () => void } | null>(null);
  // Set when another viewer decides the PTY size: this view is then letterboxed at that size.
  const [shared, setShared] = useState<{ rows: number; cols: number } | null>(null);
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
    // The absolute offset after the last output byte written to the emulator (null before the first snapshot).
    let offset: number | null = null;
    let heardAt = 0;
    const t = new Terminal({ theme: useTheme.getState().theme === "dark" ? darkTheme : lightTheme, fontFamily: '"DejaVu Sans Mono", "Menlo", "Consolas", monospace', fontSize: Math.round(12 * currentScale.current), scrollback: 5000, allowProposedApi: false });
    const fit = new FitAddon();
    t.loadAddon(fit);
    t.open(host.current);
    let owner: Owner = "self";
    let pty = { rows: t.rows, cols: t.cols };
    let sent = "";
    let claiming = false;
    const mac = /Mac/.test(navigator.platform);
    t.attachCustomKeyEventHandler((event) => {
      const key = event.key.toLowerCase();
      const command = mac && event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey;
      const control = event.ctrlKey && !event.metaKey && !event.altKey;
      const copy = key === "c" && (command || (control && event.shiftKey));
      const paste = key === "v" && (command || control);
      const clear = key === "l" && command;
      // Typing takes the PTY size; copying or pressing a modifier alone doesn't.
      if (event.type === "keydown" && !copy && !["shift", "control", "alt", "meta"].includes(key)) claim();
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
    const input = t.onData((d) => {
      // Phone key bar: a sticky Ctrl turns the next typed letter into its control character.
      if (keyBar.ctrl && d.length === 1 && /[a-z@[\\\]^_]/i.test(d)) {
        keyBar.ctrl = false;
        keyBar.changed();
        d = String.fromCharCode(d.toUpperCase().charCodeAt(0) & 0x1f);
      }
      send(encoder.encode(d));
    });
    terminalInputs.set(id, {
      input: (data, wasUserInput) => { if (!disposed) { claim(); t.input(data, wasUserInput); } },
      paste: (text) => { if (!disposed) { claim(); t.paste(text); } },
      focus: () => { if (!disposed) t.focus(); },
      getSelection: () => t.getSelection(),
      selectAll: () => t.selectAll(),
      // Respect cursor-key mode in full-screen programs as well as normal shell prompts.
      arrow: (direction) => { claim(); t.input(`\u001b${t.modes.applicationCursorKeysMode ? "O" : "["}${direction}`, true); },
    });
    const binary = t.onBinary((d) => send(Uint8Array.from(d, (c) => c.charCodeAt(0) & 0xff)));
    // The size this view would fit; the daemon applies it only while this view owns the PTY size.
    const sendSize = () => {
      if (!currentVisible.current || output?.replaying || socket?.readyState !== WebSocket.OPEN) return;
      const dims = fit.proposeDimensions();
      if (!dims || !Number.isFinite(dims.rows) || !Number.isFinite(dims.cols)) return;
      const message = JSON.stringify({ type: "resize", rows: dims.rows, cols: dims.cols });
      if (message !== sent) { sent = message; socket.send(message); }
    };
    // Owner: fit the container. Others: the PTY size, overflowing into the scrollable container.
    const refit = () => {
      if (disposed || output?.replaying || !host.current?.clientWidth || !host.current.clientHeight) return;
      const letterbox = owner !== "self";
      if (t.element) t.element.style.width = t.element.style.height = letterbox ? "max-content" : "";
      if (!letterbox) fit.fit();
      else if (t.rows !== pty.rows || t.cols !== pty.cols) t.resize(pty.cols, pty.rows);
      sendSize();
    };
    const claim = () => {
      if (owner === "self" || claiming || socket?.readyState !== WebSocket.OPEN) return;
      claiming = true;
      sendSize();
      socket.send(JSON.stringify({ type: "claim" }));
    };
    const takeSize = (control: Control) => {
      owner = control.owner;
      pty = { rows: control.rows, cols: control.cols };
      claiming = false;
      setShared(owner === "self" ? null : pty);
    };
    // Native paste (menu, middle click) and soft keyboards without useful key events.
    host.current.addEventListener("paste", claim, true);
    host.current.addEventListener("beforeinput", claim, true);
    let output: ReturnType<typeof terminalOutput> | null = null;
    instance.current = { terminal: t, refit, claim };
    const connect = () => {
      if (disposed) return;
      clearTimeout(retryTimer);
      retryTimer = undefined;
      const query = offset === null ? "" : `?offset=${offset}`;
      const ws = new WebSocket(`${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/api/terminals/${id}/ws${query}`);
      ws.binaryType = "arraybuffer";
      socket = ws;
      heardAt = Date.now();
      sent = "";
      claiming = false;
      const stream = terminalOutput(t, (epoch, bytes) => {
        if (!disposed && ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify({ type: "ack", epoch, bytes }));
      }, () => { if (!disposed && socket === ws) refit(); });
      output = stream;
      ws.onopen = () => { retry = 0; };
      ws.onmessage = (msg) => {
        if (disposed || socket !== ws) return;
        heardAt = Date.now();
        if (typeof msg.data !== "string") {
          const data = new Uint8Array(msg.data as ArrayBuffer);
          // Counted when handed to xterm: queued writes are parsed even if the socket drops.
          if (offset !== null) offset += data.byteLength;
          stream.data(data);
          return;
        }
        // `exit` also arrives on the bus, which updates the tab.
        const control = JSON.parse(msg.data) as Control;
        if (control.type === "reset" || control.type === "resume") {
          takeSize(control);
          sent = "";
          offset = control.offset;
          if (control.type === "reset") stream.reset(control.epoch, control.rows, control.cols);
          else stream.resume(control.epoch, control.rows, control.cols);
        }
        if (control.type === "ready") stream.ready();
        // After the output that was written at the previous size.
        if (control.type === "size") t.write("", () => { if (!disposed && socket === ws) { takeSize(control); refit(); } });
      };
      // Retry until the tab is removed (the terminal list refresh does that when it no longer exists).
      ws.onclose = () => {
        if (disposed || socket !== ws) return;
        socket = null;
        retryTimer = setTimeout(connect, Math.min(500 * 2 ** retry, MAX_RETRY_MS));
        retry = Math.min(retry + 1, 10);
      };
    };
    /** Drops the current socket without waiting for its close handshake, then connects again. */
    const restart = () => {
      const old = socket;
      socket = null;
      if (old) { old.onclose = null; old.close(); }
      retry = 0;
      connect();
    };
    const check = () => {
      if (disposed) return;
      if (!socket) { retry = 0; connect(); }
      else if (socket.readyState === WebSocket.CONNECTING) { if (Date.now() - heardAt > 5_000) restart(); }
      else if (socket.readyState !== WebSocket.OPEN || Date.now() - heardAt > STALE_MS) restart();
    };
    const onVisible = () => { if (document.visibilityState === "visible") check(); };
    const watchdog = setInterval(() => { if (socket?.readyState === WebSocket.OPEN && Date.now() - heardAt > STALE_MS) restart(); }, 5_000);
    document.addEventListener("visibilitychange", onVisible);
    window.addEventListener("online", check);
    window.addEventListener("pageshow", check);
    connect();
    const observer = new ResizeObserver(refit);
    observer.observe(host.current);
    return () => { terminalInputs.delete(id); disposed = true; clearTimeout(retryTimer); clearInterval(watchdog); document.removeEventListener("visibilitychange", onVisible); window.removeEventListener("online", check); window.removeEventListener("pageshow", check); instance.current = null; observer.disconnect(); input.dispose(); binary.dispose(); socket?.close(); t.dispose(); };
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
    <div className="flex min-h-0 flex-1 flex-col" style={{ display: visible ? undefined : "none" }}>
      {shared && (
        <div className="flex items-center gap-2 border-b border-line bg-paper px-2 py-0.5 text-[12px] text-ink-2" role="status">
          <span>Sized for another device ({shared.cols} × {shared.rows}). Typing here also resizes it.</span>
          <button className="btn btn-sm" title="Resize the terminal to fit this window" onClick={() => { instance.current?.claim(); instance.current?.terminal.focus(); }}>Use this size</button>
        </div>
      )}
      <div className={`min-h-0 flex-1 bg-paper px-2 py-1 ${shared ? "overflow-auto" : "overflow-hidden"}`} style={{ zoom: 1 / scale }}>
        <div ref={host} className="h-full w-full" />
      </div>
    </div>
  );
}
