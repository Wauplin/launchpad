// Oracles part of the demo world (Home's `oracles`, `oracle_eval`, `gates`): four oracles of Acme (one failing,
// with its Today row), their evaluations and versions, the Maintainer's routine gated on CI, and an oracle card from
// Ask Launchpad that adds a tool. The demo never runs Python: Test answers from the oracle's last outcome.

import type { Author } from "@/gen/Author";
import type { Change } from "@/gen/Change";
import type { CronAgent } from "@/gen/CronAgent";
import type { Oracle } from "@/gen/Oracle";
import type { OracleEvaluation } from "@/gen/OracleEvaluation";
import type { OracleMeta } from "@/gen/OracleMeta";
import type { OracleOutcome } from "@/gen/OracleOutcome";
import type { OracleStreak } from "@/gen/OracleStreak";
import type { Proposal } from "@/gen/Proposal";
import type { RoutineRun } from "@/gen/RoutineRun";
import { accessOf, paramsOf } from "@/lib/oracles";
import { clock } from "../daemon";
import { defaultSpec } from "../store";
import { BOT } from "./bots";
import { P, WS } from "./ids";

/** A demo oracle: its record, file, evaluations (newest first), versions and failure streak. */
export type DemoOracle = { oracle: Oracle; content: string; evaluations: OracleEvaluation[]; versions: (Change & { patch: string })[]; streak: OracleStreak | null };

/** The tools granted to Acme, as `GET /api/oracles/{workspace}/_tools` lists them. */
export const ORACLE_TOOLS = [
  { id: "gh", name: "GitHub CLI", env: ["GH_TOKEN"], hosts: ["api.github.com", "github.com"] },
  { id: "hf", name: "Hugging Face CLI", env: ["HF_TOKEN"], hosts: ["huggingface.co"] },
];

export const GATED = "demo-routine-fix-ci";

const CI_RED = `# /// script
# requires-python = ">=3.12"
# dependencies = []
#
# [tool.launchpad]
# title = "CI red on a branch"
# description = "True when the latest completed run of a workflow failed."
# params = { repo = "repo", branch = "string:main", workflow = "string:tests.yml" }
# min_interval = "2m"
# access = { tools = ["gh"] }
# ///
import os
from launchpad_oracle import Context, run, verdict

RUNS = "https://api.github.com/repos/{}/actions/workflows/{}/runs"


def check(ctx: Context):
    p = ctx.params
    r = ctx.http.get(RUNS.format(p.repo, p.workflow),
                     params={"branch": p.branch, "status": "completed", "per_page": 1},
                     headers={"Authorization": "Bearer " + os.environ["GH_TOKEN"]})
    r.raise_for_status()
    runs = r.json()["workflow_runs"]
    if not runs:
        return verdict(False, summary="no run yet")
    last = runs[0]
    return verdict(last["conclusion"] == "failure",
                   summary=f"{p.workflow} on {last['head_sha'][:7]}: {last['conclusion']}",
                   fingerprint=last["head_sha"], links=[last["html_url"]])


run(check)
`;

const NEW_ITEMS = `# /// script
# requires-python = ">=3.11"
# dependencies = []
#
# [tool.launchpad]
# title = "New PRs and issues"
# description = "True when the repository has pull requests or issues newer than the cursor."
# params = { repo = "repo" }
# access = { tools = ["gh"] }
# ///
from launchpad_oracle import Context, run, verdict


def check(ctx: Context):
    since = ctx.cursor or "1970-01-01T00:00:00Z"
    r = ctx.http.get(f"https://api.github.com/repos/{ctx.params.repo}/issues", params={"since": since, "state": "open", "sort": "created"})
    r.raise_for_status()
    items = [i for i in r.json() if i["created_at"] > since]
    newest = max((i["created_at"] for i in items), default=since)
    return verdict(bool(items), summary=f"{len(items)} new", items=[{"number": i["number"], "title": i["title"]} for i in items], cursor=newest)


run(check)
`;

const PYPI = `# /// script
# requires-python = ">=3.11"
# dependencies = ["packaging>=24"]
#
# [tool.launchpad]
# title = "Newer release on PyPI"
# description = "True when a package has a release newer than the version we pin."
# params = { package = "string", version = "string", pre = "bool:false" }
# access = { hosts = ["pypi.org"] }
# ///
from packaging.version import Version
from launchpad_oracle import Context, run, verdict


def check(ctx: Context):
    p = ctx.params
    r = ctx.http.get(f"https://pypi.org/pypi/{p.package}/json")
    r.raise_for_status()
    versions = [Version(v) for v in r.json()["releases"]]
    newest = max(v for v in versions if p.pre or not v.is_prerelease)
    return verdict(newest > Version(p.version), summary=f"{p.package} {newest} is the newest", fingerprint=str(newest))


run(check)
`;

const BACKLOG = `# /// script
# requires-python = ">=3.11"
# dependencies = []
#
# [tool.launchpad]
# title = "Done agents waiting"
# description = "True when more agents wait for your look than the threshold."
# params = { threshold = "int:5" }
# access = { launchpad = "read" }
# ///
from launchpad_oracle import Context, run, verdict


def check(ctx: Context):
    lp = ctx.launchpad
    today = ctx.http.get(f"{lp['url']}/api/today", headers={"Authorization": "Bearer " + lp["token"]}).json()
    n = sum(1 for i in today["items"] if i["kind"] == "finished")
    return verdict(n > ctx.params.threshold, summary=f"{n} Done and unseen (threshold {ctx.params.threshold})")


run(check)
`;

/** Pure: an oracle file's metadata as Home parses it, for the demo's few inline forms. */
export function metaOf(content: string): OracleMeta | null {
  const value = (key: string) => content.match(new RegExp(`^#\\s*${key}\\s*=\\s*"([^"]*)"`, "m"))?.[1];
  const title = value("title");
  if (!title) return null;
  const deps = content.match(/^#\s*dependencies\s*=\s*\[([^\]]*)\]/m)?.[1] ?? "";
  const seconds = (v: string | undefined, d: number) => { const m = v?.match(/^(\d+)(s|m|h)?$/); return m ? Number(m[1]) * (m[2] === "h" ? 3600 : m[2] === "m" ? 60 : 1) * 1000 : d; };
  return { title, description: value("description") ?? null, requires_python: value("requires-python") ?? null, dependencies: [...deps.matchAll(/"([^"]+)"/g)].map((m) => m[1]), params: paramsOf(content).map((p) => ({ ...p, default: p.default == null ? null : p.kind === "bool" ? p.default === "true" : p.kind === "int" ? Number(p.default) : p.default })), min_interval_ms: seconds(value("min_interval"), 60_000), timeout_ms: seconds(value("timeout"), 10_000), access: accessOf(content) ?? { tools: [], hosts: [], launchpad: "none" } };
}

/** A unified diff that adds a whole file. */
export const added = (text: string) => { const lines = text.trimEnd().split("\n"); return `@@ -0,0 +1,${lines.length} @@\n${lines.map((l) => `+${l}`).join("\n")}\n`; };

const you: Author = { kind: "owner", name: "You", session_id: null, model: null, runner: null };
const ask: Author = { kind: "agent", name: "Ask Launchpad", session_id: "demo-s-ask-acme", model: null, runner: null };

function oracle(slug: string, content: string, createdMin: number, changedMin: number, by = you, paused = false): Oracle {
  return { workspace_id: WS.acme, slug, sha256: `demo-${slug}-${content.length}`, meta: metaOf(content), error: null, created_by: ask, created_at: clock.ago(createdMin), changed_by: by, changed_at: clock.ago(changedMin), paused };
}

type Ev = { min: number; outcome: OracleOutcome; summary?: string; fp?: string | null; error?: OracleEvaluation["error"]; caller?: string; consequence?: string | null; detail?: string | null; ms?: number };

function evals(slug: string, params: Record<string, unknown>, list: Ev[]): OracleEvaluation[] {
  return list.map((e, i) => ({
    id: `ev_demo_${slug}_${i}`, workspace_id: WS.acme, oracle: slug, file_sha: `demo-${slug}`, params, event: { kind: e.caller === "test" ? "owner" : "schedule" },
    cursor_key: e.caller ? null : `routine:${GATED}`, cursor_in: null, cursor_proposed: null, outcome: e.outcome, error: e.error ?? null, summary: e.summary ?? "", fingerprint: e.fp ?? null,
    links: e.outcome === "true" && slug === "ci-red" ? ["https://github.com/acme-labs/storefront/actions/runs/9120331"] : [], items: [], stderr_tail: e.error?.class === "http" ? "httpx2.HTTPStatusError: Client error '401 Unauthorized' for url 'https://api.github.com/repos/acme-labs/storefront/issues'\n" : "",
    duration_ms: e.ms ?? 300 + ((i * 37) % 200), created_at: clock.ago(e.min),
    callers: [{ caller: e.caller === "test" ? { kind: "test", id: null, name: "Test now" } : { kind: "routine", id: e.caller ?? GATED, name: e.caller ?? "Fix red CI" }, at: clock.ago(e.min), how: "started", consequence: e.consequence ?? null, detail: e.detail ?? null }],
    reused: false,
  }));
}

/** Every ten minutes since `from` minutes ago: quiet False checks of the gated routine. */
const quietFalse = (from: number, to: number): Ev[] => Array.from({ length: Math.max(0, Math.floor((from - to) / 10)) }, (_, i) => ({ min: to + i * 10, outcome: "false" as const, summary: "tests.yml on 91ac03e: success", fp: "91ac03e", consequence: "quiet", detail: "not routed" }));

function version(slug: string, id: number, min: number, by: Author, before: string | null, after: string, patch: string): Change & { patch: string } {
  return { id, path: `oracles/${slug}.py`, sha256: before, new_sha256: `v${id}`, at: clock.ago(min), session_id: by.session_id, author: by, added: after.split("\n").length, removed: before ? 1 : 0, patch };
}

export function oraclesWorld() {
  const day = 60 * 24;
  const ciEvals = evals("ci-red", { repo: "acme-labs/storefront", branch: "main", workflow: "tests.yml" }, [
    { min: 6, outcome: "true", summary: "tests.yml on 4be1c2a: failure", fp: "4be1c2a", consequence: "quiet", detail: "no change" },
    { min: 16, outcome: "true", summary: "tests.yml on 4be1c2a: failure", fp: "4be1c2a", consequence: "fired", detail: "run started: new fingerprint" },
    { min: 26, outcome: "failed", error: { class: "http", message: "502 Bad Gateway from api.github.com (GET /repos/acme-labs/storefront/actions/workflows/tests.yml/runs)" }, consequence: "counted", ms: 5100 },
    ...quietFalse(180, 36),
  ]);
  const newEvals = evals("new-items", { repo: "acme-labs/storefront" }, Array.from({ length: 7 }, (_, i) => ({ min: 4 + i * 15, outcome: "failed" as const, error: { class: "http" as const, message: "401 Unauthorized from api.github.com (GET /repos/acme-labs/storefront/issues): Bad credentials" }, caller: "Triage", consequence: "counted" })));
  const pypiEvals = evals("pypi-release", { package: "acme-sdk", version: "2.3.0" }, [
    { min: 120, outcome: "false", summary: "acme-sdk 2.3.0 is the newest", fp: "2.3.0", caller: "test", consequence: "returned" },
    { min: 60 * 26, outcome: "true", summary: "acme-sdk 2.4.0rc1 is the newest", fp: "2.4.0rc1", caller: "test", consequence: "returned" },
  ]);
  const backlogEvals = evals("done-backlog", { threshold: 5 }, [{ min: 11, outcome: "false", summary: "3 Done and unseen (threshold 5)", caller: "test", consequence: "returned" }]);
  const oracles: DemoOracle[] = [
    { oracle: oracle("ci-red", CI_RED, day * 4, day * 2), content: CI_RED, evaluations: ciEvals, streak: { workspace_id: WS.acme, oracle: "ci-red", count: 0, first_at: null, last_at: null, classes: {}, last_error: null, last_good_at: clock.ago(6), read_at: null, recovered_at: clock.ago(16), recovered_after: 1 },
      versions: [
        version("ci-red", 2, day * 2, you, "v1", CI_RED, "@@ -9,1 +9,1 @@\n-# min_interval = \"1m\"\n+# min_interval = \"2m\"\n"),
        version("ci-red", 1, day * 4, ask, null, CI_RED, added(CI_RED)),
      ] },
    { oracle: oracle("new-items", NEW_ITEMS, day * 9, day * 9, ask), content: NEW_ITEMS, evaluations: newEvals, streak: { workspace_id: WS.acme, oracle: "new-items", count: 7, first_at: clock.ago(94), last_at: clock.ago(4), classes: { http: 7 }, last_error: { class: "http", message: "401 Unauthorized from api.github.com: Bad credentials" }, last_good_at: clock.ago(day), read_at: null, recovered_at: null, recovered_after: 0 },
      versions: [version("new-items", 3, day * 9, ask, null, NEW_ITEMS, added(NEW_ITEMS))] },
    { oracle: oracle("pypi-release", PYPI, day * 12, day * 3), content: PYPI, evaluations: pypiEvals, streak: null,
      versions: [version("pypi-release", 4, day * 12, you, null, PYPI, added(PYPI))] },
    { oracle: oracle("done-backlog", BACKLOG, day * 2, day * 2), content: BACKLOG, evaluations: backlogEvals, streak: null,
      versions: [version("done-backlog", 5, day * 2, you, null, BACKLOG, added(BACKLOG))] },
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

const MODEL_CARD = `# /// script
# requires-python = ">=3.11"
# dependencies = []
#
# [tool.launchpad]
# title = "Model card changed"
# description = "True when the model card of a Hub repository changed since the cursor."
# params = { repo = "string" }
# access = { tools = ["hf"] }
# ///
import os
from launchpad_oracle import Context, run, verdict


def check(ctx: Context):
    r = ctx.http.get(f"https://huggingface.co/api/models/{ctx.params.repo}", headers={"Authorization": "Bearer " + os.environ["HF_TOKEN"]})
    r.raise_for_status()
    sha = r.json()["sha"]
    return verdict(ctx.cursor is not None and sha != ctx.cursor, summary=f"{ctx.params.repo} at {sha[:7]}", fingerprint=sha, cursor=sha)


run(check)
`;

/** Ask Launchpad's oracle card: a new oracle that adds the Hugging Face CLI tool, so only you can accept it. */
export function oracleCard(): Proposal {
  return {
    id: "demo-prop-oracle", workspace_id: WS.acme, session_id: "demo-s-ask-acme", title: "New oracle: Model card changed",
    note: "Tested in my jail with my session's Hugging Face token: False (acme-labs/storefront-embeddings at 3f2a91c). It adds the Hugging Face CLI tool, so it waits for your click.",
    body: { kind: "oracle", slug: "model-card", content: MODEL_CARD, before: null, patch: added(MODEL_CARD), access_added: ["tool hf"], access_removed: [] },
    state: "proposed", result_id: null, replaced_by: null, error: null, created_at: clock.ago(9), updated_at: clock.ago(9),
  };
}
