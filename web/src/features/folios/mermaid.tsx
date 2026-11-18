// The ```mermaid block (lazy-loaded with the Mermaid library, only on folios that use it): drawn in strict mode
// (no HTML labels, no click handlers), its SVG sanitized with DOMPurify. A diagram that does not parse shows its
// source with the message, reported as F702.

import { memo, useEffect, useId, useState } from "react";
import DOMPurify from "dompurify";
import mermaid from "mermaid";

let ready: string | null = null;

const MermaidView = memo(function MermaidView({ text, dark, onError }: { text: string; dark: boolean; onError: (message: string) => void }) {
  const id = `m${useId().replace(/[^a-zA-Z0-9]/g, "")}`;
  const [out, setOut] = useState<{ svg: string } | { error: string } | null>(null);
  useEffect(() => {
    let alive = true;
    const theme = dark ? "dark" : "neutral";
    if (ready !== theme) {
      mermaid.initialize({ startOnLoad: false, securityLevel: "strict", theme, htmlLabels: false, flowchart: { htmlLabels: false }, fontFamily: "inherit" });
      ready = theme;
    }
    void mermaid.render(id, text).then(({ svg }) => {
      if (alive) setOut({ svg: DOMPurify.sanitize(svg, { USE_PROFILES: { svg: true, svgFilters: true } }) });
    }).catch((e: unknown) => {
      const message = String((e as Error)?.message ?? e).split("\n").slice(0, 3).join(" ").slice(0, 300);
      document.getElementById(`d${id}`)?.remove();
      if (alive) { setOut({ error: message }); onError(message); }
    });
    return () => { alive = false; };
  }, [text, dark, id, onError]);
  if (!out) return <div className="folio-diagram folio-caption">Drawing the diagram…</div>;
  if ("error" in out) return <div className="folio-problem" role="note"><div className="folio-problem-head">This diagram cannot draw</div><div className="folio-problem-line">F702 · {out.error}</div><pre>{text}</pre></div>;
  return <div className="folio-diagram" dangerouslySetInnerHTML={{ __html: out.svg }} />;
});
export default MermaidView;
