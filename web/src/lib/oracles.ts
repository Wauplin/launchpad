// Oracles: small Python checks Home runs in Monty to answer True, False or Failed (`/api/oracles/{workspace}…`).
// Routines use them as gates; the Schedules page lists them (Schedules → Oracles).

import { useQuery } from "@tanstack/react-query";
import type { Gate } from "@/gen/Gate";
import type { Oracle } from "@/gen/Oracle";
import type { OracleDetail } from "@/gen/OracleDetail";
import type { OracleEvaluation } from "@/gen/OracleEvaluation";
import type { OracleListItem } from "@/gen/OracleListItem";
import type { OracleOutcome } from "@/gen/OracleOutcome";
import type { OracleProblem } from "@/gen/OracleProblem";
import type { OracleStreak } from "@/gen/OracleStreak";
import type { OracleToolInfo } from "@/gen/OracleToolInfo";
import type { RoutineRun } from "@/gen/RoutineRun";
import type { StatusTone } from "@/components/ui";
import { ApiError, api } from "./api";
import { daemonUrl } from "./connection";
import { when } from "./format";
import { plural } from "./oracleFile";

export * from "./oracleFile";

const ws = (w: string | null | undefined) => encodeURIComponent(w ?? "");
const slugPath = (w: string, slug: string) => `/api/oracles/${ws(w)}/${encodeURIComponent(slug)}`;

export const useOracles = (workspace: string | null | undefined) => useQuery({ queryKey: ["oracles", workspace], queryFn: () => api.get<OracleListItem[]>(`/api/oracles/${ws(workspace)}`), enabled: !!workspace, staleTime: 30_000 });
export const useOracle = (workspace: string | null | undefined, slug: string | null | undefined) => useQuery({ queryKey: ["oracle", workspace, slug], queryFn: () => api.get<OracleDetail>(slugPath(workspace ?? "", slug ?? "")), enabled: !!workspace && !!slug, staleTime: 15_000 });
export const useOracleTools = (workspace: string | null | undefined) => useQuery({ queryKey: ["oracle-tools", workspace], queryFn: () => api.get<OracleToolInfo[]>(`/api/oracles/${ws(workspace)}/_tools`), enabled: !!workspace, staleTime: 60_000 });
export const useOracleEvaluations = (workspace: string, slug: string, failed: boolean) => useQuery({ queryKey: ["oracle", workspace, slug, "evaluations", failed], queryFn: () => api.get<OracleEvaluation[]>(`${slugPath(workspace, slug)}/evaluations?limit=100${failed ? "&failed=true" : ""}`), staleTime: 15_000 });
/** The interface of `launchpad_oracle` (its `.pyi`, plain text at `OracleDetail.interface`). */
export const useOracleInterface = (url: string | null | undefined) => useQuery({
  queryKey: ["oracle-interface", url],
  queryFn: async () => { const r = await fetch(daemonUrl(url ?? ""), { credentials: "include" }); if (!r.ok) throw new ApiError(r.status, r.statusText); return r.text(); },
  enabled: !!url, staleTime: Infinity,
});

export const OUTCOME_LABEL: Record<OracleOutcome, string> = { true: "True", false: "False", failed: "Failed" };
export const OUTCOME_TONE: Record<OracleOutcome, StatusTone> = { true: "done", false: "idle", failed: "failed" };
/** A square of the recent outcomes: green True, grey False, red Failed. */
export const OUTCOME_SQUARE: Record<OracleOutcome, string> = { true: "bg-gh-open", false: "bg-ink-3/40", failed: "bg-gh-closed" };

/** The failure streak is open (a Today row): failing now, not marked read. */
export const failing = (s: OracleStreak | null | undefined) => !!s && s.count > 0 && s.read_at == null;
/** The stored file does not pass the pre-check: Home does not evaluate it until it is saved again. */
export const broken = (o: Oracle | null | undefined) => !!o && o.problems.length > 0;

/** The pre-check problems of a refused save or test (HTTP 422 `{"error", "problems"}`), else null. */
export function problemsOf(e: unknown): OracleProblem[] | null {
  if (!(e instanceof ApiError) || e.status !== 422) return null;
  const p = e.body?.problems;
  return Array.isArray(p) ? p as OracleProblem[] : null;
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
export const STARTER = `# /// oracle
# title = "New release on PyPI"
# description = "Is the newest release of a package on PyPI different from the version given?"
# tools = ["http"]
# min_interval = "5m"
# ///
from launchpad_oracle import tools, verdict

# Each tool you call is listed in the header's tools line (the Tools tab edits it):
#   tools.http.get("https://pypi.org/pypi/huggingface_hub/json")  # any public address, no credential
#   tools.github.get("/repos/acme/app/pulls", params={"state": "open"})  # GitHub's REST API, GET only
#   tools.huggingface.get("/api/models/openai-community/gpt2")  # the Hugging Face Hub, GET only
#   tools.launchpad.sessions.list()  # this workspace's Launchpad, read only


def check(package: str, version: str = "1.0.0"):
    info = tools.http.get(f"https://pypi.org/pypi/{package}/json").raise_for_status().json["info"]
    newest = info["version"]
    return verdict(newest != version, summary=f"{package}: newest is {newest}", fingerprint=newest)
`;

/** The gate in a few words: "Only when ci-red says True · when it changes". */
export function gateSummary(g: Gate): string {
  return `Only when ${g.oracle} says ${g.run_on.map((o) => OUTCOME_LABEL[o]).join(" or ") || "—"} · ${g.fire === "every" ? "every time" : g.fire === "change" ? "when it changes" : "no fire policy"}${g.cooldown ? ` · cooldown ${g.cooldown}` : ""}${g.remind ? ` · remind ${g.remind}` : ""}`;
}
