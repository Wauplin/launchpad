// Demo routes: oracles (Home's `crates/api/src/oracles.rs`), answered from the demo world. Saving and testing a draft
// run the demo's small pre-check (a refused file answers 422 with its problems, like Home); Test never runs Python: a
// saved oracle answers its last outcome, a draft a canned answer with one call per tool of its header.

import type { OracleDetail } from "@/gen/OracleDetail";
import type { OracleListItem } from "@/gen/OracleListItem";
import type { OracleProblem } from "@/gen/OracleProblem";
import type { OracleRunResult } from "@/gen/OracleRunResult";
import type { OracleSaved } from "@/gen/OracleSaved";
import type { OracleTestAnswer } from "@/gen/OracleTestAnswer";
import type { OracleUser } from "@/gen/OracleUser";
import { toolsChange } from "@/lib/oracleFile";
import { DemoError, clock, publish, route } from "../daemon";
import { world } from "../world";
import { INTERFACE, ORACLE_TOOL_INFO, added, metaOf, precheck, problemsOf, type DemoOracle } from "../world/oracles";

const of = (ws: string, slug: string): DemoOracle => {
  const o = world.oracles.find((x) => x.oracle.workspace_id === ws && x.oracle.slug === slug);
  if (!o) throw new DemoError(404, `no oracle ${slug} in this workspace`);
  return o;
};

const usedBy = (ws: string, slug: string): OracleUser[] => world.crons.filter((c) => c.gate?.oracle === slug && c.spec.workspace_id === ws).map((c) => ({ routine_id: c.id, title: c.title, bot_id: c.bot_id, run_on: c.gate!.run_on, fire: c.gate!.fire ?? null, paused: c.paused }));

const item = (o: DemoOracle): OracleListItem => ({ oracle: o.oracle, last: o.evaluations[0] ?? null, recent: o.evaluations.slice(0, 5).map((e) => ({ outcome: e.outcome, at: e.created_at })), used_by: usedBy(o.oracle.workspace_id, o.oracle.slug), streak: o.streak });

const changed = (o: DemoOracle) => publish("oracles", "changed", { workspace_id: o.oracle.workspace_id, slug: o.oracle.slug });

/** Home's answer to a file the pre-check refuses: 422 with each problem. */
const refuse = (problems: OracleProblem[]) => new Response(JSON.stringify({ error: `the pre-check refused the file: ${problems.length} error${problems.length === 1 ? "" : "s"}`, problems }), { status: 422, headers: { "Content-Type": "application/json" } });

/** The demo's type warning: `verdict("…")` with a string where a bool goes. */
const typeWarnings = (content: string): OracleProblem[] => content.split("\n").flatMap((l, i) => { const c = l.search(/verdict\(\s*["']/); return c >= 0 ? [{ line: i + 1, column: c + 9, rule: "invalid-argument-type", message: "Argument to `verdict` is incorrect: expected `bool`, found `str`" }] : []; });

route("GET", "/api/oracles/:ws", ({ params }) => world.oracles.filter((o) => o.oracle.workspace_id === params.ws).map(item));
route("GET", "/api/oracles/:ws/_tools", () => ORACLE_TOOL_INFO);
route("GET", "/api/oracles/:ws/_interface", () => new Response(INTERFACE, { headers: { "Content-Type": "text/x-python; charset=utf-8" } }));
route("GET", "/api/oracles/:ws/:slug", ({ params }): OracleDetail => { const o = of(params.ws, params.slug); return { ...item(o), content: o.content, warnings: o.oracle.problems.length ? [] : typeWarnings(o.content), interface: `/api/oracles/${encodeURIComponent(params.ws)}/_interface` }; });
route("GET", "/api/oracles/:ws/:slug/evaluations", ({ params, query }) => {
  const o = of(params.ws, params.slug);
  const list = query.get("failed") === "true" ? o.evaluations.filter((e) => e.outcome === "failed") : o.evaluations;
  return list.slice(0, Number(query.get("limit") ?? 50));
});
route("GET", "/api/oracles/:ws/:slug/versions", ({ params }) => of(params.ws, params.slug).versions.map(({ patch: _, ...c }) => c));
route("GET", "/api/oracles/:ws/:slug/versions/:change", ({ params }) => ({ patch: of(params.ws, params.slug).versions.find((v) => String(v.id) === params.change)?.patch ?? "" }));

route("PUT", "/api/oracles/:ws/:slug", ({ params, body }) => {
  const content = String(body?.content ?? "");
  if (!/^[a-z0-9][a-z0-9_-]{0,62}$/.test(params.slug)) throw new DemoError(400, "a slug uses lowercase letters, digits, - and _");
  const problems = precheck(content);
  if (problems.length) return refuse(problems);
  const meta = metaOf(content)!;
  const existing = world.oracles.find((x) => x.oracle.workspace_id === params.ws && x.oracle.slug === params.slug);
  const author = { kind: "owner" as const, name: "You", session_id: null, model: null, runner: null };
  const id = Math.max(0, ...world.oracles.flatMap((o) => o.versions.map((v) => v.id))) + 1;
  const now = clock.now();
  const change = toolsChange(existing?.oracle.meta?.tools ?? [], meta.tools);
  const answer = (o: DemoOracle): OracleSaved => ({ oracle: o.oracle, warnings: typeWarnings(content), tools_added: change.added, tools_removed: change.removed, tools_missing: meta.tools.flatMap((t) => { const i = ORACLE_TOOL_INFO.find((x) => x.tool === t); return i && !i.ready ? [`${t}: ${i.problem ?? "not ready in this workspace"}`] : []; }) });
  if (existing) {
    const before = existing.content;
    existing.versions.unshift({ id, path: `oracles/${params.slug}.py`, sha256: existing.oracle.sha256, new_sha256: `v${id}`, at: now, session_id: null, author, added: content.split("\n").length, removed: before.split("\n").length, patch: `${added(content).replace(/^@@[^\n]*\n/, `@@ -1,${before.split("\n").length} +1,${content.split("\n").length} @@\n${before.trimEnd().split("\n").map((l) => `-${l}`).join("\n")}\n`)}` });
    existing.content = content;
    existing.oracle = { ...existing.oracle, meta, problems: problemsOf(content), sha256: `demo-${params.slug}-${id}`, changed_by: author, changed_at: now };
    changed(existing);
    return answer(existing);
  }
  const o: DemoOracle = { oracle: { workspace_id: params.ws, slug: params.slug, sha256: `demo-${params.slug}-${id}`, meta, problems: [], created_by: author, created_at: now, changed_by: author, changed_at: now, paused: false }, content, evaluations: [], streak: null, versions: [{ id, path: `oracles/${params.slug}.py`, sha256: null, new_sha256: `v${id}`, at: now, session_id: null, author, added: content.split("\n").length, removed: 0, patch: added(content) }] };
  world.oracles.push(o);
  changed(o);
  return answer(o);
});

const pause = (paused: boolean) => ({ params }: { params: Record<string, string> }) => { const o = of(params.ws, params.slug); o.oracle = { ...o.oracle, paused }; changed(o); return o.oracle; };
route("POST", "/api/oracles/:ws/:slug/pause", pause(true));
route("POST", "/api/oracles/:ws/:slug/resume", pause(false));
route("POST", "/api/oracles/:ws/:slug/read", ({ params }) => { const o = of(params.ws, params.slug); if (o.streak) o.streak = { ...o.streak, read_at: clock.now() }; changed(o); return { ok: true }; });
route("POST", "/api/oracles/:ws/:slug/restore/:change", ({ params }) => { const o = of(params.ws, params.slug); changed(o); return { ok: true, change: params.change }; });
route("DELETE", "/api/oracles/:ws/:slug", ({ params }) => {
  const o = of(params.ws, params.slug);
  world.oracles = world.oracles.filter((x) => x !== o);
  changed(o);
  return { ok: true };
});

/** A draft's canned answer: one call per tool of its header. */
const DRAFT_CALLS: Record<string, string> = { http: "GET https://pypi.org/pypi/acme-sdk/json", github: "GET /repos/acme-labs/storefront/pulls?state=open", huggingface: "GET /api/models/acme-labs/storefront-embeddings", launchpad: "sessions.list()" };

/** The demo's answer: the oracle's last outcome, or for a draft a canned True with one call per tool. */
function answer(o: DemoOracle | undefined, draft: string | null): OracleRunResult {
  const last = draft == null ? o?.evaluations[0] : undefined;
  if (last) return { outcome: last.outcome, error: last.error, summary: last.summary, fingerprint: last.fingerprint, links: last.links, items: last.items, cursor: last.cursor_proposed ?? undefined, log: last.log, calls: last.calls, duration_ms: last.duration_ms };
  const tools = draft ? metaOf(draft)?.tools ?? [] : [];
  return { outcome: "true", error: null, summary: "demo: Home would run this draft in Monty", fingerprint: null, links: [], items: [], cursor: undefined, log: "demo: no Python runs in the demo\n", calls: tools.map((t) => ({ tool: t, call: DRAFT_CALLS[t] ?? t, outcome: "ok", status: 200, bytes: 1800, duration_ms: 240 })), duration_ms: 410 };
}

route("POST", "/api/oracles/:ws/:slug/test", ({ params, body }): OracleTestAnswer | Response => {
  const o = world.oracles.find((x) => x.oracle.workspace_id === params.ws && x.oracle.slug === params.slug);
  const draft = typeof body?.content === "string" ? body.content as string : null;
  if (draft != null) {
    const problems = precheck(draft);
    if (problems.length) return refuse(problems);
    return { result: answer(o, draft), evaluation_id: null, warnings: typeWarnings(draft) };
  }
  if (!o) throw new DemoError(404, `no oracle ${params.slug} in this workspace`);
  if (o.oracle.problems.length) throw new DemoError(400, `the oracle ${params.slug} is broken and does not run until it is saved again`);
  const result = answer(o, null);
  const ev = { ...o.evaluations[0], id: `ev_demo_test_${clock.now()}`, created_at: clock.now(), params: (body?.params as Record<string, unknown>) ?? o.evaluations[0]?.params ?? {}, callers: [{ caller: { kind: "test" as const, id: null, name: "Test now" }, at: clock.now(), how: "started", consequence: null, detail: null }] };
  if (o.evaluations.length) o.evaluations.unshift(ev);
  changed(o);
  return { result, evaluation_id: ev.id ?? null, warnings: [] };
});
route("POST", "/api/oracles/:ws/:slug/run", ({ params }) => of(params.ws, params.slug).evaluations[0] ?? (() => { throw new DemoError(400, "never evaluated in the demo"); })());
