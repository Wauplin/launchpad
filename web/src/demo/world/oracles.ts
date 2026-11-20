// Oracles part of the demo world (Home's `oracles`, `oracle_eval`, `gates`): Acme's oracles in the `# /// oracle`
// format (one failing, with its Today row, and one broken file in the older `# /// script` format), their
// evaluations with call logs, versions, the Maintainer's routine gated on CI, and an oracle card from Ask Launchpad
// that adds tools. The demo never runs Python: Test answers from the oracle's last outcome.

import type { Author } from "@/gen/Author";
import type { Change } from "@/gen/Change";
import type { CronAgent } from "@/gen/CronAgent";
import type { Oracle } from "@/gen/Oracle";
import type { OracleCall } from "@/gen/OracleCall";
import type { OracleEvaluation } from "@/gen/OracleEvaluation";
import type { OracleMeta } from "@/gen/OracleMeta";
import type { OracleOutcome } from "@/gen/OracleOutcome";
import type { OracleProblem } from "@/gen/OracleProblem";
import type { OracleStreak } from "@/gen/OracleStreak";
import type { OracleToolInfo } from "@/gen/OracleToolInfo";
import type { Proposal } from "@/gen/Proposal";
import type { RoutineRun } from "@/gen/RoutineRun";
import { canLeak, signatureOf, toolsOf } from "@/lib/oracleFile";
import { clock } from "../daemon";
import { defaultSpec } from "../store";
import { BOT } from "./bots";
import { P, WS } from "./ids";

/** A demo oracle: its record, file, evaluations (newest first), versions and failure streak. */
export type DemoOracle = { oracle: Oracle; content: string; evaluations: OracleEvaluation[]; versions: (Change & { patch: string })[]; streak: OracleStreak | null };

/** The four tools in Acme, as `GET /api/oracles/{workspace}/_tools` lists them. */
export const ORACLE_TOOL_INFO: OracleToolInfo[] = [
  { tool: "http", reaches: "Any public http(s) address, without a credential", registry: null, ready: true, problem: null },
  { tool: "github", reaches: "GitHub's REST API (api.github.com), GET only", registry: "gh", ready: true, problem: null },
  { tool: "huggingface", reaches: "The Hugging Face Hub (huggingface.co), GET only", registry: "hf", ready: true, problem: null },
  { tool: "launchpad", reaches: "Home's read API for this workspace, in process", registry: null, ready: true, problem: null },
];

/** The interface of `launchpad_oracle` (`GET /api/oracles/{workspace}/_interface`), shortened for the demo. */
export const INTERFACE = `# launchpad_oracle.pyi: the whole interface an oracle sees (served by Home, used for type checking)
from dataclasses import dataclass
from datetime import datetime
from typing import Any, NoReturn

JSON = dict[str, Any] | list[Any] | str | int | float | bool | None

@dataclass(frozen=True)
class Response:
    status_code: int
    text: str | None      # the body decoded as UTF-8, or None when it is not valid UTF-8
    json: JSON            # the text parsed as JSON, or None when there is no text or it is not JSON
    def raise_for_status(self) -> Response: ...   # 4xx or 5xx: the evaluation fails with class http

@dataclass(frozen=True)
class Verdict: ...

def verdict(value: bool, *, summary: str = "", fingerprint: str | None = None,
            links: list[str] | None = None, items: list[JSON] | None = None,
            cursor: str | None = None) -> Verdict: ...
def failed(reason: str) -> NoReturn: ...         # ends the evaluation with Failed (class declared)

class _Http:
    def get(self, url: str, params: dict[str, Any] | None = None, headers: dict[str, str] | None = None) -> Response: ...

class _Api:                                    # tools.github, tools.huggingface
    def get(self, endpoint: str, params: dict[str, Any] | None = None, headers: dict[str, str] | None = None) -> Response: ...

class _Launchpad: ...                          # folders, bots, sessions, schedules, proposals, today(), notes(), get

class _Tools:
    http: _Http
    github: _Api
    huggingface: _Api
    launchpad: _Launchpad

tools: _Tools
`;

export const GATED = "demo-routine-fix-ci";

const CI_RED = `# /// oracle
# title = "CI red on a branch"
# description = "Is the latest completed run of a workflow on this branch a failure?"
# tools = ["github"]
# min_interval = "2m"
# ///
from launchpad_oracle import tools, verdict


def check(repo: str, branch: str = "main", workflow: str = "tests.yml"):
    runs = (
        tools.github.get(
            f"/repos/{repo}/actions/workflows/{workflow}/runs",
            params={"branch": branch, "status": "completed", "per_page": 1},
        )
        .raise_for_status()
        .json["workflow_runs"]
    )
    if not runs:
        return verdict(False, summary=f"no completed run of {workflow} on {branch} yet")
    last = runs[0]
    return verdict(
        last["conclusion"] == "failure",
        summary=f"{workflow} on {branch} @ {last['head_sha'][:7]}: {last['conclusion']}",
        fingerprint=last["head_sha"],
        links=[last["html_url"]],
    )
`;

const NEW_ITEMS = `# /// oracle
# title = "New PRs and issues"
# description = "Has the repository received pull requests or issues since the last check?"
# tools = ["github"]
# ///
from launchpad_oracle import tools, verdict


def check(cursor: str | None, repo: str):
    since = cursor or "1970-01-01T00:00:00Z"
    found = (
        tools.github.get(f"/repos/{repo}/issues", params={"since": since, "state": "open", "sort": "created"})
        .raise_for_status()
        .json
    )
    items = [i for i in found if i["created_at"] > since]
    newest = max([i["created_at"] for i in items], default=since)
    return verdict(
        len(items) > 0,
        summary=f"{len(items)} new",
        items=[{"number": i["number"], "title": i["title"]} for i in items],
        cursor=newest,
    )
`;

const WAITING = `# /// oracle
# title = "Agents waiting on me"
# description = "Has an agent of this workspace waited for me for more than N hours?"
# tools = ["launchpad"]
# ///
from datetime import datetime, timezone

from launchpad_oracle import tools, verdict


def check(now: datetime, hours: float = 2):
    cutoff = now.timestamp() * 1000 - hours * 3600 * 1000
    stale = [s for s in tools.launchpad.sessions.list() if s["needs_you"] and s["status_since"] < cutoff]
    stale.sort(key=lambda s: s["status_since"])
    return verdict(
        len(stale) > 0,
        summary=f"{len(stale)} agent(s) waiting more than {hours:g} h",
        items=[
            {
                "title": s["title"],
                "since": datetime.fromtimestamp(s["status_since"] / 1000, timezone.utc).isoformat(),
            }
            for s in stale
        ],
        fingerprint=",".join([s["id"] for s in stale]),
    )
`;

/** An oracle of the older format: Home no longer runs it until it is saved again in the new one. */
const LEGACY = `# /// script
# requires-python = ">=3.11"
# dependencies = ["packaging>=24"]
#
# [tool.launchpad]
# title = "Newer release on PyPI"
# description = "True when a package has a release newer than the version we pin."
# params = { package = "string", version = "string" }
# access = { hosts = ["pypi.org"] }
# ///
from packaging.version import Version
from launchpad_oracle import Context, run, verdict


def check(ctx: Context):
    p = ctx.params
    r = ctx.http.get(f"https://pypi.org/pypi/{p.package}/json")
    r.raise_for_status()
    newest = max(Version(v) for v in r.json()["releases"])
    return verdict(newest > Version(p.version), summary=f"{p.package} {newest} is the newest", fingerprint=str(newest))


run(check)
`;

const LEGACY_PROBLEMS: OracleProblem[] = [{ line: 1, column: 1, rule: "header", message: "the file starts with `# /// script`: an oracle now starts with `# /// oracle` (title, description, tools); parameters are the arguments of check" }];

/** Pure: the pre-check of the demo, for the few mistakes it simulates (Home runs Ruff and Monty): the header, the
 * tools, `check`, and the name `tool` instead of `tools`. Empty: it passes. */
export function precheck(content: string): OracleProblem[] {
  const lines = content.split("\n");
  if (!/^#\s*\/\/\/\s*oracle\s*$/.test(lines[0] ?? "")) return [{ line: 1, column: 1, rule: "header", message: "an oracle starts with a `# /// oracle` block (title, description, tools)" }];
  if (!/^#\s*title\s*=\s*"[^"]+"/m.test(content)) return [{ line: 1, column: 1, rule: "header", message: "the header has no title" }];
  const tools = toolsOf(content);
  const unknown = tools?.find((t) => !ORACLE_TOOL_INFO.some((x) => x.tool === t));
  if (tools == null || unknown) return [{ line: lines.findIndex((l) => /^#\s*tools\s*=/.test(l)) + 1, column: 1, rule: "header", message: unknown ? `unknown tool "${unknown}": tools are http, github, huggingface and launchpad` : "write the tools as one list: tools = [\"github\"]" }];
  const out: OracleProblem[] = [];
  lines.forEach((l, i) => { const c = l.search(/\btool\./); if (c >= 0 && !l.trimStart().startsWith("#")) out.push({ line: i + 1, column: c + 1, rule: "F821", message: "Undefined name `tool`" }); });
  if (!signatureOf(content)) out.push({ line: null, column: null, rule: "signature", message: "the file defines no `def check(...)`" });
  return out;
}

/** Pure: an oracle file's metadata as Home reads it (header and signature), for the demo's forms. Null: broken. */
export function metaOf(content: string): OracleMeta | null {
  if (precheck(content).length) return null;
  const value = (key: string) => content.match(new RegExp(`^#\\s*${key}\\s*=\\s*"([^"]*)"`, "m"))?.[1];
  const sig = signatureOf(content)!;
  const ms = (v: string | undefined, d: number) => { const m = v?.match(/^(\d+)(ms|s|m|h)?$/); return m ? Number(m[1]) * (m[2] === "h" ? 3600_000 : m[2] === "m" ? 60_000 : m[2] === "ms" ? 1 : 1000) : d; };
  return { title: value("title")!, description: value("description") ?? null, tools: toolsOf(content) ?? [], min_interval_ms: ms(value("min_interval"), 60_000), timeout_ms: ms(value("timeout"), 10_000), params: sig.params, cursor: sig.cursor, now: sig.now };
}

/** Pure: why a stored file is broken (empty: it is not). */
export const problemsOf = (content: string): OracleProblem[] => (/^#\s*\/\/\/\s*script/.test(content) ? LEGACY_PROBLEMS : precheck(content));

/** A unified diff that adds a whole file. */
export const added = (text: string) => { const lines = text.trimEnd().split("\n"); return `@@ -0,0 +1,${lines.length} @@\n${lines.map((l) => `+${l}`).join("\n")}\n`; };

const you: Author = { kind: "owner", name: "You", session_id: null, model: null, runner: null };
const ask: Author = { kind: "agent", name: "Ask Launchpad", session_id: "demo-s-ask-acme", model: null, runner: null };

function oracle(slug: string, content: string, createdMin: number, changedMin: number, by = you, paused = false): Oracle {
  return { workspace_id: WS.acme, slug, sha256: `demo-${slug}-${content.length}`, meta: metaOf(content), problems: problemsOf(content), created_by: ask, created_at: clock.ago(createdMin), changed_by: by, changed_at: clock.ago(changedMin), paused };
}

const call = (tool: string, what: string, outcome = "ok", bytes = 2140, ms = 380): OracleCall => ({ tool, call: what, outcome, status: /^ok$/.test(outcome) ? 200 : Number(outcome.match(/\d{3}/)?.[0] ?? 0) || null, bytes, duration_ms: ms });

type Ev = { min: number; outcome: OracleOutcome; summary?: string; fp?: string | null; error?: OracleEvaluation["error"]; caller?: string; consequence?: string | null; detail?: string | null; ms?: number; calls?: OracleCall[]; log?: string; links?: string[]; items?: unknown[]; cursor?: string };

function evals(slug: string, params: Record<string, unknown>, list: Ev[]): OracleEvaluation[] {
  return list.map((e, i) => ({
    id: `ev_demo_${slug}_${i}`, workspace_id: WS.acme, oracle: slug, file_sha: `demo-${slug}`, params, event: { kind: e.caller === "test" ? "owner" : "schedule" },
    cursor_key: e.caller ? null : `routine:${GATED}`, cursor_in: null, cursor_proposed: e.cursor ?? null, outcome: e.outcome, error: e.error ?? null, summary: e.summary ?? "", fingerprint: e.fp ?? null,
    links: e.links ?? [], items: e.items ?? [], log: e.log ?? "", calls: e.calls ?? [],
    duration_ms: e.ms ?? 300 + ((i * 37) % 200), created_at: clock.ago(e.min),
    callers: [{ caller: e.caller === "test" ? { kind: "test", id: null, name: "Test now" } : { kind: "routine", id: e.caller ?? GATED, name: e.caller ?? "Fix red CI" }, at: clock.ago(e.min), how: "started", consequence: e.consequence ?? null, detail: e.detail ?? null }],
    reused: false,
  }));
}

const RUNS_CALL = "GET /repos/acme-labs/storefront/actions/workflows/tests.yml/runs?branch=main&status=completed&per_page=1";

/** Every ten minutes since `from` minutes ago: quiet False checks of the gated routine. */
const quietFalse = (from: number, to: number): Ev[] => Array.from({ length: Math.max(0, Math.floor((from - to) / 10)) }, (_, i) => ({ min: to + i * 10, outcome: "false" as const, summary: "tests.yml on main @ 91ac03e: success", fp: "91ac03e", consequence: "quiet", detail: "not routed", calls: [call("github", RUNS_CALL, "ok", 2310, 290)], links: ["https://github.com/acme-labs/storefront/actions/runs/9120310"] }));

function version(slug: string, id: number, min: number, by: Author, before: string | null, after: string, patch: string): Change & { patch: string } {
  return { id, path: `oracles/${slug}.py`, sha256: before, new_sha256: `v${id}`, at: clock.ago(min), session_id: by.session_id, author: by, added: after.split("\n").length, removed: before ? 1 : 0, patch };
}

const ISSUES_CALL = "GET /repos/acme-labs/storefront/issues?since=2026-11-16T09:12:00Z&state=open&sort=created";

export function oraclesWorld() {
  const day = 60 * 24;
  const red = { summary: "tests.yml on main @ 4be1c2a: failure", fp: "4be1c2a", calls: [call("github", RUNS_CALL, "ok", 2280, 310)], links: ["https://github.com/acme-labs/storefront/actions/runs/9120331"] };
  const ciEvals = evals("ci-red", { repo: "acme-labs/storefront", branch: "main", workflow: "tests.yml" }, [
    { min: 6, outcome: "true", ...red, consequence: "quiet", detail: "no change" },
    { min: 16, outcome: "true", ...red, consequence: "fired", detail: "run started: new fingerprint" },
    { min: 26, outcome: "failed", error: { class: "http", message: "HTTP 502", call: 0 }, calls: [call("github", RUNS_CALL, "HTTP 502", 157, 5020)], consequence: "counted", ms: 5100 },
    ...quietFalse(180, 36),
  ]);
  const newEvals = evals("new-items", { repo: "acme-labs/storefront" }, Array.from({ length: 7 }, (_, i) => ({ min: 4 + i * 15, outcome: "failed" as const, error: { class: "http" as const, message: "HTTP 401", call: 0 }, calls: [call("github", ISSUES_CALL, "HTTP 401", 131, 240)], log: "since = 2026-11-16T09:12:00Z\n", caller: "Triage", consequence: "counted" })));
  const waitingEvals = evals("agents-waiting", { hours: 2 }, [
    { min: 11, outcome: "false", summary: "0 agent(s) waiting more than 2 h", fp: "", caller: "test", consequence: "returned", calls: [call("launchpad", "sessions.list()", "ok", 18_400, 12)] },
  ]);
  const legacyEvals = evals("pypi-release", { package: "acme-sdk", version: "2.3.0" }, [
    { min: 60 * 26, outcome: "false", summary: "acme-sdk 2.3.0 is the newest", fp: "2.3.0", caller: "test", consequence: "returned" },
  ]);
  const oracles: DemoOracle[] = [
    { oracle: oracle("ci-red", CI_RED, day * 4, day * 2), content: CI_RED, evaluations: ciEvals, streak: { workspace_id: WS.acme, oracle: "ci-red", count: 0, first_at: null, last_at: null, classes: {}, last_error: null, last_call: null, last_good_at: clock.ago(6), read_at: null, recovered_at: clock.ago(16), recovered_after: 1 },
      versions: [
        version("ci-red", 2, day * 2, you, "v1", CI_RED, "@@ -4,1 +4,1 @@\n-# min_interval = \"1m\"\n+# min_interval = \"2m\"\n"),
        version("ci-red", 1, day * 4, ask, null, CI_RED, added(CI_RED)),
      ] },
    { oracle: oracle("new-items", NEW_ITEMS, day * 9, day * 9, ask), content: NEW_ITEMS, evaluations: newEvals, streak: { workspace_id: WS.acme, oracle: "new-items", count: 7, first_at: clock.ago(94), last_at: clock.ago(4), classes: { http: 7 }, last_error: { class: "http", message: "HTTP 401", call: 0 }, last_call: `github ${ISSUES_CALL} · HTTP 401`, last_good_at: clock.ago(day), read_at: null, recovered_at: null, recovered_after: 0 },
      versions: [version("new-items", 3, day * 9, ask, null, NEW_ITEMS, added(NEW_ITEMS))] },
    { oracle: oracle("agents-waiting", WAITING, day * 2, day * 2), content: WAITING, evaluations: waitingEvals, streak: null,
      versions: [version("agents-waiting", 5, day * 2, you, null, WAITING, added(WAITING))] },
    { oracle: oracle("pypi-release", LEGACY, day * 12, day * 12), content: LEGACY, evaluations: legacyEvals, streak: null,
      versions: [version("pypi-release", 4, day * 12, you, null, LEGACY, added(LEGACY))] },
  ];
  return { oracles };
}

/** The Maintainer's routine gated on ci-red: every 10 minutes, a run only when CI turns red (and a repair run when
 * the oracle fails), 143 quiet checks a day. */
export function gatedRoutine(): CronAgent {
  const run = (min: number, outcome: RoutineRun["outcome"], extra: Partial<RoutineRun> = {}): RoutineRun => ({ at: clock.ago(min), cause: "Scheduled", session_id: null, outcome, detail: null, ...extra });
  return {
    id: GATED, title: "Fix red CI", schedule: "*/10 * * * *", trigger: { kind: "schedule" }, bot_id: BOT, bot_paused: false, paused: false, created_at: clock.ago(60 * 24 * 4),
    spec: defaultSpec({ harness: "claude", profile: "claude-max", target: { kind: "project", project_id: P.storefront }, text: "CI on main is red (see the oracle evidence). Find the failing job, fix it on a branch and open a Local PR.", workspace_id: WS.acme }),
    conditions: [], next_run_at: clock.ago(-4), last_check_at: clock.ago(6), last_outcome: "Checked", runs: ["demo-s-bot-maintainer"], failure_seen: null, channel_seen: null, file: "routines/fix-red-ci.toml", file_error: null, auto_archive: false, waiting: null,
    gate: { oracle: "ci-red", params: { repo: "acme-labs/storefront", branch: "main" }, run_on: ["true", "failed"], fire: "change", cooldown: "6h", failed: { instructions: "The oracle could not decide. Diagnose why and fix it." } },
    history: [
      run(180, "checked", { check: { oracle: "ci-red", outcome: "false", evaluation_id: null, summary: "tests.yml on 91ac03e: success", fingerprint: "91ac03e", error: null, why: "not routed", count: 143, since: clock.ago(60 * 24) } }),
      run(26, "checked", { check: { oracle: "ci-red", outcome: "failed", evaluation_id: "ev_demo_ci-red_2", summary: "", fingerprint: null, error: "http: 502 Bad Gateway from api.github.com", why: "once per failure streak: the next good answer ends it", count: 1, since: clock.ago(26) } }),
      run(16, "report", { session_id: "demo-s-bot-maintainer", check: { oracle: "ci-red", outcome: "true", evaluation_id: "ev_demo_ci-red_1", summary: "tests.yml on 4be1c2a: failure", fingerprint: "4be1c2a", error: null, why: "new fingerprint", count: 1, since: clock.ago(16) } }),
      run(6, "checked", { check: { oracle: "ci-red", outcome: "true", evaluation_id: "ev_demo_ci-red_0", summary: "tests.yml on 4be1c2a: failure", fingerprint: "4be1c2a", error: null, why: "no change", count: 1, since: clock.ago(6) } }),
    ],
  };
}

const PYPI_CARD = `# /// oracle
# title = "PyPI release without an issue"
# description = "Is the newest PyPI release of a package missing from the repository's issues?"
# tools = ["http", "github"]
# min_interval = "15m"
# ///
from launchpad_oracle import tools, verdict


def check(package: str, repo: str):
    version = tools.http.get(f"https://pypi.org/pypi/{package}/json").raise_for_status().json["info"]["version"]
    found = (
        tools.github.get(
            "/search/issues",
            params={"q": f'repo:{repo} "{version}" in:title,body', "per_page": 1},
        )
        .raise_for_status()
        .json
    )
    return verdict(
        found["total_count"] == 0,
        summary=f"{package} {version}: {found['total_count']} issue(s) mention it",
        fingerprint=version,
        links=[f"https://pypi.org/project/{package}/{version}/"],
    )
`;

/** Ask Launchpad's oracle card: a new oracle with http and github (it could send what it reads anywhere), so only you
 * can accept it. */
export function oracleCard(): Proposal {
  const tools = toolsOf(PYPI_CARD) ?? [];
  return {
    id: "demo-prop-oracle", workspace_id: WS.acme, session_id: "demo-s-ask-acme", title: "New oracle: PyPI release without an issue",
    note: "Tested on Home with `launchpad oracle test`: False (acme-sdk 2.4.0: 3 issues mention it). It uses github, which my session does not hold, so it waits for your click.",
    body: { kind: "oracle", slug: "pypi-issue", content: PYPI_CARD, before: null, patch: added(PYPI_CARD), tools, tools_added: tools, tools_removed: [], can_leak: canLeak(tools), warnings: [{ line: 21, column: 9, rule: "invalid-argument-type", message: "`found[\"total_count\"]` may be `None`: compare it with `== 0` only after checking it" }] },
    state: "proposed", result_id: null, replaced_by: null, error: null, created_at: clock.ago(9), updated_at: clock.ago(9),
  };
}
