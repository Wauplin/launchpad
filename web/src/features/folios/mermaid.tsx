// The ```mermaid block (lazy-loaded with the Mermaid library, only on folios that use it): drawn in strict mode
// (no HTML labels, no click handlers), its SVG sanitized with DOMPurify and kept off the network (`cleanSvg`). A diagram that does not parse shows its
// source with the message, reported as F702.

import { memo, useEffect, useId, useState } from "react";
import DOMPurify from "dompurify";
import mermaid from "mermaid";

let ready: string | null = null;

/** The diagram's SVG with nothing that reaches the network (it sits in Launchpad's page, unlike a sketch): no images
 * or foreign objects, links and `url(…)` only to the SVG's own `#ids`, no `@import`. */
function cleanSvg(svg: string): string {
  const root = DOMPurify.sanitize(svg, { USE_PROFILES: { svg: true, svgFilters: true }, FORBID_TAGS: ["image", "feImage", "foreignObject"], RETURN_DOM: true }) as HTMLElement;
  // Links and references stay inside the SVG (`#id`).
  for (const el of root.querySelectorAll("*")) {
    for (const name of ["href", "xlink:href"]) if (el.hasAttribute(name) && !el.getAttribute(name)!.trim().startsWith("#")) el.removeAttribute(name);
  }
  return root.innerHTML.replace(/@import[^;]*;?/gi, "").replace(/url\(\s*(?!['"]?#)[^)]*\)/gi, "none");
}

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
      if (alive) setOut({ svg: cleanSvg(svg) });
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
