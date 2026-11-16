// Oracles: small Python checks Home runs to answer True, False or Failed (`/api/oracles/{workspace}…`). Routines use
// them as gates; the Schedules page lists them (Schedules → Oracles).

import { useQuery } from "@tanstack/react-query";
import type { Gate } from "@/gen/Gate";
import type { OracleAccess } from "@/gen/OracleAccess";
import type { OracleDetail } from "@/gen/OracleDetail";
import type { OracleEvaluation } from "@/gen/OracleEvaluation";
import type { OracleListItem } from "@/gen/OracleListItem";
import type { OracleMeta } from "@/gen/OracleMeta";
import type { OracleOutcome } from "@/gen/OracleOutcome";
import type { OracleStreak } from "@/gen/OracleStreak";
import type { RoutineRun } from "@/gen/RoutineRun";
import type { StatusTone } from "@/components/ui";
import { api } from "./api";
import { when } from "./format";

/** A tool granted to the workspace, as an oracle may name it (`GET /api/oracles/{workspace}/_tools`). */
export type OracleTool = { id: string; name: string; env: string[]; hosts: string[] };

const ws = (w: string | null | undefined) => encodeURIComponent(w ?? "");
const slugPath = (w: string, slug: string) => `/api/oracles/${ws(w)}/${encodeURIComponent(slug)}`;

export const useOracles = (workspace: string | null | undefined) => useQuery({ queryKey: ["oracles", workspace], queryFn: () => api.get<OracleListItem[]>(`/api/oracles/${ws(workspace)}`), enabled: !!workspace, staleTime: 30_000 });
export const useOracle = (workspace: string | null | undefined, slug: string | null | undefined) => useQuery({ queryKey: ["oracle", workspace, slug], queryFn: () => api.get<OracleDetail>(slugPath(workspace ?? "", slug ?? "")), enabled: !!workspace && !!slug, staleTime: 15_000 });
export const useOracleTools = (workspace: string | null | undefined) => useQuery({ queryKey: ["oracle-tools", workspace], queryFn: () => api.get<OracleTool[]>(`/api/oracles/${ws(workspace)}/_tools`), enabled: !!workspace, staleTime: 60_000 });
export const useOracleEvaluations = (workspace: string, slug: string, failed: boolean) => useQuery({ queryKey: ["oracle", workspace, slug, "evaluations", failed], queryFn: () => api.get<OracleEvaluation[]>(`${slugPath(workspace, slug)}/evaluations?limit=100${failed ? "&failed=true" : ""}`), staleTime: 15_000 });

export const OUTCOME_LABEL: Record<OracleOutcome, string> = { true: "True", false: "False", failed: "Failed" };
export const OUTCOME_TONE: Record<OracleOutcome, StatusTone> = { true: "done", false: "idle", failed: "failed" };
/** A square of the recent outcomes: green True, grey False, red Failed. */
export const OUTCOME_SQUARE: Record<OracleOutcome, string> = { true: "bg-gh-open", false: "bg-ink-3/40", failed: "bg-gh-closed" };

/** The failure streak is open (a Today row): failing now, not marked read. */
export const failing = (s: OracleStreak | null | undefined) => !!s && s.count > 0 && s.read_at == null;

/** What an oracle reaches, in a few words: "GitHub CLI · pypi.org · Launchpad read", or "no access". */
export function accessSummary(a: OracleAccess | null | undefined, tools: OracleTool[] = []): string {
  if (!a) return "no access";
  const parts = [
    ...a.tools.map((id) => tools.find((t) => t.id === id)?.name ?? id),
    ...a.hosts.map((h) => (h === "*" ? "every host" : h)),
    ...(a.launchpad === "read" ? ["Launchpad read"] : []),
  ];
  return parts.length ? parts.join(" · ") : "no access";
}

export const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`;

/** The dependencies in a few words: "no dependencies", "1 dependency". */
export const dependencySummary = (m: OracleMeta | null | undefined) => (!m || !m.dependencies.length ? "no dependencies" : m.dependencies.length === 1 ? "1 dependency" : `${m.dependencies.length} dependencies`);

/** The parameters as written: "repo (repo, required)", "branch = main". */
export function paramLabel(p: OracleMeta["params"][number]): string {
  return p.default == null ? `${p.name} · ${p.kind}, required` : `${p.name} · ${p.kind} = ${typeof p.default === "string" ? p.default : JSON.stringify(p.default)}`;
}

/** The newly added and removed access between two versions (Home's `oracles::widens`, in words). */
export function accessChange(before: OracleAccess | null | undefined, after: OracleAccess | null | undefined): { added: string[]; removed: string[] } {
  const words = (a: OracleAccess | null | undefined) => new Set([...(a?.tools ?? []).map((t) => `tool ${t}`), ...(a?.hosts ?? []).map((h) => `host ${h}`), ...(a?.launchpad === "read" ? ["Launchpad read"] : [])]);
  const b = words(before), n = words(after);
  return { added: [...n].filter((x) => !b.has(x)), removed: [...b].filter((x) => !n.has(x)) };
}

/** Pure: the access of a file's `access = { … }` line in its PEP 723 block (what the editor shows before saving).
 * Null when the file has no such line (no access) or writes it another way. */
export function accessOf(src: string): OracleAccess | null {
  const line = src.split("\n").find((l) => /^#\s*access\s*=\s*\{/.test(l));
  if (!line) return null;
  const list = (key: string) => {
    const m = line.match(new RegExp(`\\b${key}\\s*=\\s*\\[([^\\]]*)\\]`));
    return m ? [...m[1].matchAll(/"([^"]*)"/g)].map((x) => x[1]) : [];
  };
  return { tools: list("tools"), hosts: list("hosts"), launchpad: /\blaunchpad\s*=\s*"read"/.test(line) ? "read" : "none" };
}

/** Pure: the parameters of a file's `params = { … }` line (what Test asks for before the draft is saved). */
export function paramsOf(src: string): OracleMeta["params"] {
  const line = src.split("\n").find((l) => /^#\s*params\s*=\s*\{/.test(l));
  if (!line) return [];
  return [...line.matchAll(/([a-z_][a-z0-9_]*)\s*=\s*"([a-z]+)(?::([^"]*))?"/g)].map((m) => ({ name: m[1], kind: m[2] as OracleMeta["params"][number]["kind"], default: m[3] ?? null }));
}

/** Pure: the file with its access line replaced (or added before the block's end), or null when the block writes its
 * access another way (an `[tool.launchpad.access]` table): then the owner edits the file. */
export function withAccess(src: string, a: OracleAccess): string | null {
  const lines = src.split("\n");
  if (lines.some((l) => /^#\s*\[tool\.launchpad\.access\]/.test(l))) return null;
  const q = (xs: string[]) => `[${xs.map((x) => JSON.stringify(x)).join(", ")}]`;
  const parts = [a.tools.length ? `tools = ${q(a.tools)}` : "", a.hosts.length ? `hosts = ${q(a.hosts)}` : "", a.launchpad === "read" ? `launchpad = "read"` : ""].filter(Boolean);
  const text = parts.length ? `# access = { ${parts.join(", ")} }` : null;
  const at = lines.findIndex((l) => /^#\s*access\s*=/.test(l));
  if (at >= 0) {
    if (!lines[at].includes("}")) return null;
    if (text) lines[at] = text; else lines.splice(at, 1);
    return lines.join("\n");
  }
  if (!text) return src;
  const start = lines.findIndex((l) => /^#\s*\/\/\/\s*script\s*$/.test(l));
  const end = start < 0 ? -1 : lines.findIndex((l, i) => i > start && /^#\s*\/\/\/\s*$/.test(l));
  if (end < 0 || !lines.slice(start, end).some((l) => /^#\s*\[tool\.launchpad\]/.test(l))) return null;
  lines.splice(end, 0, text);
  return lines.join("\n");
}

/** A quiet check of a gated routine in its history: "143 checks · False since 08:00 · no change". */
export function checkSummary(r: RoutineRun): string {
  const c = r.check;
  if (!c) return "Checked";
  const what = c.outcome ? OUTCOME_LABEL[c.outcome] : "no answer";
  const head = c.count > 1 ? `${plural(c.count, "check")} · ${what} since ${when(c.since)}` : `Check · ${what}`;
  return [head, c.why, c.error ?? (c.summary || null)].filter(Boolean).join(" · ");
}

/** A new oracle: the starter file of the New oracle form (the owner edits it, then saves). */
export const STARTER = `# /// script
# requires-python = ">=3.11"
# dependencies = []
#
# [tool.launchpad]
# title = "Newer release on PyPI"
# description = "True when the package has a release newer than the version given."
# params = { package = "string", version = "string" }
# access = { hosts = ["pypi.org"] }
# ///
from launchpad_oracle import Context, run, verdict


def check(ctx: Context):
    p = ctx.params
    r = ctx.http.get(f"https://pypi.org/pypi/{p.package}/json")
    r.raise_for_status()
    latest = r.json()["info"]["version"]
    return verdict(latest != p.version, summary=f"{p.package} {latest} is the newest", fingerprint=latest)


run(check)
`;
/** The gate in a few words: "Only when ci-red says True · when it changes". */
export function gateSummary(g: Gate): string {
  return `Only when ${g.oracle} says ${g.run_on.map((o) => OUTCOME_LABEL[o]).join(" or ") || "—"} · ${g.fire === "every" ? "every time" : g.fire === "change" ? "when it changes" : "no fire policy"}${g.cooldown ? ` · cooldown ${g.cooldown}` : ""}${g.remind ? ` · remind ${g.remind}` : ""}`;
}
