// A sketch block in the reader: agent-written HTML, CSS and JavaScript in a sandboxed frame (lib/folioSketch.ts:
// opaque origin, strict CSP, no network), framed as a phone, a laptop or a panel. Its only way out is `folio.data`,
// checked here (the frame's own window, its key prefix, the limits) before the reader calls Home. Script errors,
// blocked requests and empty frames become render problems (F701) of the rev.

import { useEffect, useMemo, useRef, useState } from "react";
import type { FolioFiles } from "@/lib/folios";
import { BRIDGE_CALLS, BRIDGE_WINDOW_MS, THEME_VARS, refuse, relativeSources, sketchDoc, type BridgeCall } from "@/lib/folioSketch";
import { useTheme } from "@/stores/theme";

const FRAME: Record<string, { width: number | null; height: number }> = { phone: { width: 390, height: 720 }, laptop: { width: null, height: 520 }, panel: { width: 360, height: 560 }, none: { width: null, height: 360 } };

/** `report(message)` records an F701 render problem of `file`. */
export function SketchFrame({ html, kit, file, prefix, frame, height, title, files, readOnly, report }: {
  html: string; kit: string | null; file: string; prefix: string; frame: string; height: number | null; title: string;
  files: FolioFiles; readOnly: boolean; report: (message: string) => void;
}) {
  const ref = useRef<HTMLIFrameElement>(null);
  const theme = useTheme((s) => s.theme);
  const token = useMemo(() => Array.from(crypto.getRandomValues(new Uint32Array(4)), (n) => n.toString(16)).join(""), []);
  const [inlined, setInlined] = useState<string | null>(null);
  const [autoHeight, setAutoHeight] = useState<number | null>(null);
  const f = FRAME[frame] ?? FRAME.none;
  // Images of the folio become data URLs: the frame has no network, not even Home.
  useEffect(() => {
    let alive = true;
    const dir = file.includes("/") ? file.split("/").slice(0, -1) : [];
    const resolve = (p: string) => { const parts = [...dir]; for (const s of p.replace(/^\.\//, "").split("/")) { if (s === "..") parts.pop(); else if (s && s !== ".") parts.push(s); } return parts.join("/"); };
    const sources = relativeSources(html);
    void Promise.all(sources.map(async (s) => [s, await files.dataUrl(resolve(s)).catch(() => null)] as const)).then((pairs) => {
      if (!alive) return;
      let out = html;
      for (const [s, url] of pairs) if (url) out = out.split(`"${s}"`).join(`"${url}"`).split(`'${s}'`).join(`'${url}'`);
      setInlined(out);
    });
    return () => { alive = false; };
  }, [html, file, files]);
  const doc = useMemo(() => {
    if (inlined == null) return null;
    const style = getComputedStyle(document.documentElement);
    const vars = Object.fromEntries(THEME_VARS.map((k) => [k, style.getPropertyValue(k).trim()]));
    return sketchDoc({ html: inlined, kit, theme: vars, prefix, token });
  }, [inlined, kit, prefix, token, theme]); // eslint-disable-line react-hooks/exhaustive-deps
  // The bridge: only messages of this frame's window with its token; a few calls per second at most.
  useEffect(() => {
    const calls: number[] = [];
    const onMessage = async (e: MessageEvent) => {
      const win = ref.current?.contentWindow;
      if (!win || e.source !== win) return;
      const m = e.data as Omit<Partial<BridgeCall>, "op"> & { op?: string; message?: string; empty?: boolean; height?: number };
      if (!m || m.lp !== token) return;
      if (m.op === "error") { report(String(m.message ?? "script error")); return; }
      if (m.op === "loaded") {
        if (m.empty) report("the frame stayed empty");
        if (!height && typeof m.height === "number" && f.width == null) setAutoHeight(Math.min(1200, Math.max(120, m.height)));
        return;
      }
      const reply = (r: { value?: unknown; error?: string }) => win.postMessage({ lp: token, id: m.id, ...r }, "*");
      const now = Date.now();
      while (calls.length && now - calls[0] > BRIDGE_WINDOW_MS) calls.shift();
      if (calls.length >= BRIDGE_CALLS) { reply({ error: "too many data calls: at most 60 every 10 seconds" }); return; }
      calls.push(now);
      const why = refuse(m as unknown as BridgeCall, prefix, readOnly || !files.data);
      if (why || !files.data) { reply({ error: why ?? "no data API here: a shared snapshot shows the sketch's default state" }); return; }
      try {
        const call = m as unknown as BridgeCall;
        if (call.op === "get") reply({ value: await files.data.get(call.key) });
        else if (call.op === "set") { await files.data.set(call.key, call.value); reply({ value: true }); }
        else if (call.op === "delete") { await files.data.delete(call.key); reply({ value: true }); }
        else reply({ value: await files.data.list(call.key || prefix) });
      } catch (err) { reply({ error: (err as Error).message ?? "the data call failed" }); }
    };
    const listener = (e: MessageEvent) => void onMessage(e);
    window.addEventListener("message", listener);
    return () => window.removeEventListener("message", listener);
  }, [token, prefix, readOnly, files, report, height, f.width]);
  const h = height ?? autoHeight ?? f.height;
  return <figure className={`folio-sketch frame-${frame in FRAME ? frame : "none"}`}>
    <div className="folio-sketch-device" style={f.width ? { maxWidth: f.width } : undefined}>
      {doc != null && <iframe ref={ref} title={title || `Sketch ${file}`} sandbox="allow-scripts" referrerPolicy="no-referrer" srcDoc={doc} style={{ height: h }} />}
    </div>
    {title && <figcaption className="folio-caption">{title}</figcaption>}
  </figure>;
}
