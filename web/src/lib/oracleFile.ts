// Pure helpers of oracle files (no imports but types, so `tests/` runs them with Node): the header's tools line, the
// signature of `check`, parameter values and pre-check problems.

import type { OracleParam } from "../gen/OracleParam";
import type { OracleProblem } from "../gen/OracleProblem";
import type { OracleTool } from "../gen/OracleTool";

export const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`;

/** The four tools, in the order of the Tools tab. */
export const ORACLE_TOOLS: OracleTool[] = ["http", "github", "huggingface", "launchpad"];
/** The tools that use one of the owner's tokens. */
export const CREDENTIALED: OracleTool[] = ["github", "huggingface"];
/** A credentialed tool together with http: the oracle can send what it reads to any public address. */
export const canLeak = (tools: readonly string[]) => tools.includes("http") && tools.some((t) => (CREDENTIALED as string[]).includes(t));
export const LEAK_WARNING = "can send what it reads to any public address";

/** "oracle.py:14:9: F821 Undefined name `tool`" (Home's `Problem::text`). */
export function problemText(p: OracleProblem): string {
  const at = p.line != null ? `:${p.line}${p.column != null ? `:${p.column}` : ""}` : "";
  return `oracle.py${at}: ${p.rule ? `${p.rule} ` : ""}${p.message}`;
}

/** A parameter's value as written: "main", 20, True, None. */
export const pyValue = (v: unknown) => (v === null || v === undefined ? "None" : v === true ? "True" : v === false ? "False" : typeof v === "string" ? JSON.stringify(v) : String(v));

/** The parameter as the signature writes it: "repo: str", "branch: str = \"main\"", "label: str | None = None". */
export function paramLabel(p: OracleParam): string {
  return `${p.name}: ${p.kind}${p.optional ? " | None" : ""}${p.required ? "" : ` = ${pyValue(p.default)}`}`;
}

/** Pure: a typed value from a parameter field's text. Empty: undefined (the default applies; None for an optional
 * parameter without one). Throws with a message for a value of the wrong type. */
export function paramValue(p: OracleParam, text: string): unknown {
  const t = text.trim();
  if (!t) return p.optional && p.required ? null : undefined;
  if (p.optional && (t === "None" || t === "null")) return null;
  if (p.kind === "int") { if (!/^-?\d+$/.test(t)) throw new Error(`${p.name} takes a whole number`); return Number(t); }
  if (p.kind === "float") { const n = Number(t); if (!Number.isFinite(n)) throw new Error(`${p.name} takes a number`); return n; }
  if (p.kind === "bool") return t === "true" || t === "True";
  return text;
}

/** Pure: a literal default as Python writes it (None, True, 20, 2.5, "main"), or undefined when it is not one. */
function literal(src: string): unknown {
  const t = src.trim();
  if (t === "None") return null;
  if (t === "True" || t === "False") return t === "True";
  if (/^-?\d+(\.\d*)?([eE][-+]?\d+)?$/.test(t)) return Number(t);
  const q = t.match(/^(["'])(.*)\1$/s);
  if (q) { try { return q[1] === '"' ? JSON.parse(t) : q[2].replace(/\\'/g, "'"); } catch { return q[2]; } }
  return undefined;
}

/** Pure: the arguments of a file's `def check(...)`, as Home reads them at the pre-check (best effort: Home's answer
 * is the truth). Used by Test for a draft before it is saved. Null when there is no `check`. */
export function signatureOf(src: string): { params: OracleParam[]; cursor: boolean; now: boolean } | null {
  const m = src.match(/^def check\(([\s\S]*?)\)\s*(?:->[^:\n]*)?:/m);
  if (!m) return null;
  const args: string[] = [];
  let depth = 0, quote = "", cur = "";
  for (const ch of m[1]) {
    if (quote) { cur += ch; if (ch === quote) quote = ""; continue; }
    if (ch === '"' || ch === "'") quote = ch;
    else if ("([{".includes(ch)) depth++;
    else if (")]}".includes(ch)) depth--;
    else if (ch === "," && depth === 0) { args.push(cur); cur = ""; continue; }
    cur += ch;
  }
  args.push(cur);
  const out = { params: [] as OracleParam[], cursor: false, now: false };
  for (const raw of args.map((a) => a.replace(/#[^\n]*/g, "").trim()).filter((a) => a && a !== "*" && a !== "/")) {
    const a = raw.match(/^([A-Za-z_]\w*)\s*(?::\s*([^=]+?))?\s*(?:=\s*([\s\S]+))?$/);
    if (!a) continue;
    const [, name, ann = "", def] = a;
    if (name === "cursor") { out.cursor = true; continue; }
    if (name === "now") { out.now = true; continue; }
    const kind = (["bool", "int", "float", "str"] as const).find((k) => new RegExp(`\\b${k}\\b`).test(ann)) ?? "str";
    const optional = /\bNone\b|Optional\[/.test(ann);
    out.params.push({ name, kind, optional, required: def == null, default: def == null ? null : literal(def) ?? null });
  }
  return out;
}

/** The block of the header: the indexes of its `# /// oracle` and `# ///` lines, or null. */
function block(lines: string[]): { start: number; end: number } | null {
  const start = lines.findIndex((l) => /^#\s*\/\/\/\s*oracle\s*$/.test(l));
  if (start < 0) return null;
  const end = lines.findIndex((l, i) => i > start && /^#\s*\/\/\/\s*$/.test(l));
  return end < 0 ? null : { start, end };
}

/** Pure: the tools of a file's header (`# tools = [...]`, one line), or null when the header is missing or writes
 * them another way. */
export function toolsOf(src: string): OracleTool[] | null {
  const lines = src.split("\n");
  const b = block(lines);
  if (!b) return null;
  const at = lines.findIndex((l, i) => i > b.start && i < b.end && /^#\s*tools\s*=/.test(l));
  if (at < 0) return [];
  const m = lines[at].match(/^#\s*tools\s*=\s*\[([^\]]*)\]\s*$/);
  if (!m) return null;
  return [...m[1].matchAll(/["']([^"']*)["']/g)].map((x) => x[1] as OracleTool);
}

/** Pure: the file with its header's tools line written for `tools` (added after the title and description lines
 * when missing, removed when empty), or null when the header is missing or writes the tools another way. */
export function withTools(src: string, tools: OracleTool[]): string | null {
  if (toolsOf(src) == null) return null;
  const lines = src.split("\n");
  const b = block(lines)!;
  const ordered = ORACLE_TOOLS.filter((t) => tools.includes(t));
  const text = `# tools = [${ordered.map((t) => JSON.stringify(t)).join(", ")}]`;
  const at = lines.findIndex((l, i) => i > b.start && i < b.end && /^#\s*tools\s*=/.test(l));
  if (at >= 0) {
    if (ordered.length) lines[at] = text; else lines.splice(at, 1);
    return lines.join("\n");
  }
  if (!ordered.length) return src;
  let after = b.start;
  for (let i = b.start + 1; i < b.end; i++) if (/^#\s*(title|description)\s*=/.test(lines[i])) after = i;
  lines.splice(after + 1, 0, text);
  return lines.join("\n");
}

/** The tools added and removed between two lists. */
export function toolsChange(before: readonly OracleTool[], after: readonly OracleTool[]): { added: OracleTool[]; removed: OracleTool[] } {
  return { added: after.filter((t) => !before.includes(t)), removed: before.filter((t) => !after.includes(t)) };
}
