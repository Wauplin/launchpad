// Sketch frames of folios (pure, tested by `pnpm test:folios`): the document a sketch frame shows and the checks of
// its data bridge. A sketch is agent-written HTML, CSS and JavaScript: the reader shows it only in an
// `<iframe sandbox="allow-scripts" srcdoc>` (an opaque origin: no cookies, no Launchpad API, no access to the page),
// with a strict Content-Security-Policy (no network at all; images, fonts and media only as data: or blob: URLs).
// The only way out is `folio.data`, a bridge of postMessage calls that the reader checks (the frame's own window,
// the sketch's key prefix, the limits of the data API) before it calls Home.

/** No network: inline scripts and styles, data: and blob: images, fonts and media. */
export const SKETCH_CSP = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; media-src data: blob:; connect-src 'none'; form-action 'none'; base-uri 'none'; frame-src 'none'; worker-src 'none'";

/** The limits of the data API (Home checks them again). */
export const DATA_VALUE_MAX = 64 * 1024;
export const BRIDGE_CALLS = 60;
export const BRIDGE_WINDOW_MS = 10_000;

/** The data key prefix of a sketch, like Home's `folio_check::sketch_prefix`. */
export function sketchPrefix(data: string | null | undefined, src: string | null | undefined, id: string | null | undefined): string {
  if (data) return data;
  const base = src ? (src.split("/").pop() ?? "").replace(/\.html$/, "") : id || "sketch";
  return `${base}:`;
}

/** A call of the frame's bridge. */
export type BridgeCall = { lp: string; id: number; op: "get" | "set" | "delete" | "list"; key: string; value?: unknown };

/** Pure: why the reader refuses a bridge call, or null. `readOnly`: an older rev or a shared view (reads only). */
export function refuse(call: BridgeCall, prefix: string, readOnly: boolean): string | null {
  if (!["get", "set", "delete", "list"].includes(call.op)) return `unknown call ${String(call.op)}`;
  if (typeof call.key !== "string") return "a key is text";
  if (call.op === "list" ? !call.key.startsWith(prefix) && call.key !== "" : !call.key.startsWith(prefix)) return `the key ${call.key} is outside this sketch's prefix ${prefix}`;
  if (call.op !== "list" && !/^[A-Za-z0-9_.:/-]{1,200}$/.test(call.key)) return "a key is 1 to 200 letters, digits and _ . : / -";
  if ((call.op === "set" || call.op === "delete") && readOnly) return "this view is read-only: data changes only on the latest rev";
  if (call.op === "set") {
    let text: string | undefined;
    try { text = JSON.stringify(call.value); } catch { return "the value is not JSON"; }
    if (text === undefined) return "the value is not JSON";
    if (text.length > DATA_VALUE_MAX) return "the value is over 64 KB";
  }
  return null;
}

/** The script that runs first in the frame: `folio.data` and the reports of errors and empty frames. */
function bridge(token: string, prefix: string): string {
  return `(() => {
  const token = ${JSON.stringify(token)}; let n = 0; const waiting = new Map();
  const post = (m) => parent.postMessage(Object.assign({ lp: token }, m), "*");
  addEventListener("message", (e) => { if (e.source !== parent) return; const m = e.data; if (!m || m.lp !== token || !waiting.has(m.id)) return; const [ok, ko] = waiting.get(m.id); waiting.delete(m.id); m.error ? ko(new Error(m.error)) : ok(m.value); });
  const call = (op, key, value) => new Promise((ok, ko) => { const id = ++n; waiting.set(id, [ok, ko]); post({ id, op, key: String(key ?? ""), value }); });
  window.folio = Object.freeze({ prefix: ${JSON.stringify(prefix)}, data: Object.freeze({ get: (k) => call("get", k), set: (k, v) => call("set", k, v), delete: (k) => call("delete", k), list: (p) => call("list", p ?? ${JSON.stringify(prefix)}) }) });
  const report = (message) => post({ op: "error", message: String(message).slice(0, 300) });
  addEventListener("error", (e) => report(e.message || "script error"));
  addEventListener("unhandledrejection", (e) => report((e.reason && e.reason.message) || String(e.reason)));
  document.addEventListener("securitypolicyviolation", (e) => report("blocked by the sandbox (no network): " + (e.blockedURI || e.violatedDirective)));
  addEventListener("load", () => setTimeout(() => {
    const b = document.body; const empty = !b || (b.innerText.trim() === "" && !b.querySelector("img,svg,canvas,video,input,button"));
    post({ op: "loaded", empty, height: Math.ceil(document.documentElement.scrollHeight) });
  }, 250));
})();`;
}

/** The built-in sketch kit: Launchpad-like parts (frames, bars, rows, cards, buttons, chips, pills) on the theme's
 * variables, so sketches match the app without copying its CSS. */
export const SKETCH_KIT_CSS = `
*,*::before,*::after{box-sizing:border-box}
html,body{margin:0;background:var(--color-paper);color:var(--color-ink);font:13px/1.45 var(--font-ui);}
body{padding:12px}
.lp-bar{display:flex;align-items:center;gap:8px;min-height:44px;padding:0 12px;border-bottom:1px solid var(--color-line);background:var(--color-chrome);font-weight:600}
.lp-row{display:flex;align-items:center;gap:8px;min-height:40px;padding:6px 12px;border-bottom:1px solid var(--color-line-2)}
.lp-row:hover{background:var(--color-chrome)}
.lp-card{border:1px solid var(--color-line);border-radius:10px;background:var(--color-paper);padding:10px 12px}
.lp-panel{border:1px solid var(--color-line);border-radius:10px;background:var(--color-chrome);padding:8px}
.lp-btn{display:inline-flex;align-items:center;gap:6px;height:28px;padding:0 10px;border:1px solid var(--color-line);border-radius:6px;background:var(--color-paper);color:var(--color-ink);font:inherit;cursor:pointer}
.lp-btn.primary{background:var(--color-teal);border-color:var(--color-teal);color:var(--color-paper)}
.lp-chip{display:inline-flex;align-items:center;gap:4px;height:24px;padding:0 8px;border:1px solid var(--color-line);border-radius:999px;font-size:12px;color:var(--color-ink-2)}
.lp-chip.on{border-color:var(--color-teal);color:var(--color-teal-ink);background:var(--color-teal-soft)}
.lp-pill{display:inline-block;padding:0 6px;border-radius:999px;font-size:11px;background:var(--color-sunk);color:var(--color-ink-2)}
.lp-pill.attn{background:var(--color-attn-soft);color:var(--color-attn)}
.lp-muted{color:var(--color-ink-3);font-size:12px}
.lp-title{font-size:15px;font-weight:600}
.lp-stack{display:grid;gap:8px}
.lp-grow{flex:1;min-width:0}
input,textarea,select{font:inherit;color:inherit;background:var(--color-paper);border:1px solid var(--color-line);border-radius:6px;padding:4px 8px}
`;

/** The theme's variables a sketch gets, read from the page (`:root` custom properties). */
export const THEME_VARS = ["--color-paper", "--color-chrome", "--color-sunk", "--color-ink", "--color-ink-2", "--color-ink-3", "--color-line", "--color-line-2", "--color-teal", "--color-teal-ink", "--color-teal-soft", "--color-attn", "--color-attn-soft", "--color-danger", "--font-ui", "--font-mono"];

const esc = (s: string) => s.replace(/</g, "\\3c ");

/** Pure: the document of a sketch frame: CSP first, the theme's values, the built-in kit, the bridge, the folio's
 * own kit (`sketches/_kit.html`), then the sketch. */
export function sketchDoc({ html, kit, theme, prefix, token }: { html: string; kit: string | null; theme: Record<string, string>; prefix: string; token: string }): string {
  const vars = Object.entries(theme).filter(([k, v]) => /^--[a-z0-9-]+$/.test(k) && v).map(([k, v]) => `${k}:${esc(v)}`).join(";");
  const head = `<meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${SKETCH_CSP}"><meta name="viewport" content="width=device-width,initial-scale=1"><style>:root{${vars}}${SKETCH_KIT_CSS}</style><script>${bridge(token, prefix)}</script>${kit ?? ""}`;
  // A whole document keeps its own body: the head parts go first, before anything of the sketch.
  return /<html[\s>]/i.test(html) ? `<!doctype html>${head}${html.replace(/<!doctype[^>]*>/i, "")}` : `<!doctype html><html><head>${head}</head><body>${html}</body></html>`;
}

/** Relative image sources of a sketch (`src="assets/x.png"`), which the reader inlines as data URLs. */
export function relativeSources(html: string): string[] {
  const out = new Set<string>();
  for (const m of html.matchAll(/\bsrc\s*=\s*["']([^"':]+?\.(?:png|jpe?g|gif|webp|svg))["']/gi)) out.add(m[1]);
  return [...out];
}
