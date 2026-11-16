// Demo routes: oracles (Home's `crates/api/src/oracles.rs`), answered from the demo world. Saving parses the file's
// PEP 723 block like Home for the forms the demo uses; Test never runs Python: it answers from the last outcome.

import type { OracleDetail } from "@/gen/OracleDetail";
import type { OracleListItem } from "@/gen/OracleListItem";
import type { OracleRunResult } from "@/gen/OracleRunResult";
import type { OracleTestAnswer } from "@/gen/OracleTestAnswer";
import type { OracleUser } from "@/gen/OracleUser";
import { DemoError, clock, publish, route } from "../daemon";
import { world } from "../world";
import { ORACLE_TOOLS, added, metaOf, type DemoOracle } from "../world/oracles";

const of = (ws: string, slug: string): DemoOracle => {
  const o = world.oracles.find((x) => x.oracle.workspace_id === ws && x.oracle.slug === slug);
  if (!o) throw new DemoError(404, `no oracle ${slug} in this workspace`);
  return o;
};

const usedBy = (ws: string, slug: string): OracleUser[] => world.crons.filter((c) => c.gate?.oracle === slug && c.spec.workspace_id === ws).map((c) => ({ routine_id: c.id, title: c.title, bot_id: c.bot_id, run_on: c.gate!.run_on, fire: c.gate!.fire ?? null, paused: c.paused }));

const item = (o: DemoOracle): OracleListItem => ({ oracle: o.oracle, last: o.evaluations[0] ?? null, recent: o.evaluations.slice(0, 5).map((e) => ({ outcome: e.outcome, at: e.created_at })), used_by: usedBy(o.oracle.workspace_id, o.oracle.slug), streak: o.streak });

const changed = (o: DemoOracle) => publish("oracles", "changed", { workspace_id: o.oracle.workspace_id, slug: o.oracle.slug });

route("GET", "/api/oracles/:ws", ({ params }) => world.oracles.filter((o) => o.oracle.workspace_id === params.ws).map(item));
route("GET", "/api/oracles/:ws/_tools", () => ORACLE_TOOLS);
route("GET", "/api/oracles/:ws/:slug", ({ params }): OracleDetail => { const o = of(params.ws, params.slug); return { ...item(o), content: o.content }; });
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
  const meta = metaOf(content);
  if (!meta) throw new DemoError(400, "an oracle starts with a PEP 723 block (`# /// script`) whose [tool.launchpad] table has a title");
  const unknown = meta.access.tools.find((t) => !ORACLE_TOOLS.some((x) => x.id === t));
  if (unknown) throw new DemoError(400, `there is no tool ${unknown} in Home's registry`);
  const existing = world.oracles.find((x) => x.oracle.workspace_id === params.ws && x.oracle.slug === params.slug);
  const author = { kind: "owner" as const, name: "You", session_id: null, model: null, runner: null };
  const id = Math.max(0, ...world.oracles.flatMap((o) => o.versions.map((v) => v.id))) + 1;
  const now = clock.now();
  if (existing) {
    const before = existing.content;
    existing.versions.unshift({ id, path: `oracles/${params.slug}.py`, sha256: existing.oracle.sha256, new_sha256: `v${id}`, at: now, session_id: null, author, added: content.split("\n").length, removed: before.split("\n").length, patch: `${added(content).replace(/^@@[^\n]*\n/, `@@ -1,${before.split("\n").length} +1,${content.split("\n").length} @@\n${before.trimEnd().split("\n").map((l) => `-${l}`).join("\n")}\n`)}` });
    existing.content = content;
    existing.oracle = { ...existing.oracle, meta, sha256: `demo-${params.slug}-${id}`, changed_by: author, changed_at: now };
    changed(existing);
    return existing.oracle;
  }
  const o: DemoOracle = { oracle: { workspace_id: params.ws, slug: params.slug, sha256: `demo-${params.slug}-${id}`, meta, error: null, created_by: author, created_at: now, changed_by: author, changed_at: now, paused: false }, content, evaluations: [], streak: null, versions: [{ id, path: `oracles/${params.slug}.py`, sha256: null, new_sha256: `v${id}`, at: now, session_id: null, author, added: content.split("\n").length, removed: 0, patch: added(content) }] };
  world.oracles.push(o);
  changed(o);
  return o.oracle;
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

/** The demo's answer: the oracle's last outcome (a draft answers True), recorded for a saved oracle like Home's Test. */
function answer(o: DemoOracle | undefined, draft: boolean): OracleRunResult {
  const last = draft ? undefined : o?.evaluations[0];
  const base = last ?? { outcome: "true" as const, error: null, summary: "demo: this draft would run with uv on Home", fingerprint: null, links: [], items: [], stderr_tail: "" };
  return { outcome: base.outcome, error: base.error, summary: base.summary, fingerprint: base.fingerprint, links: base.links, items: base.items, cursor: null, stderr_tail: base.stderr_tail, duration_ms: 380 };
}

route("POST", "/api/oracles/:ws/:slug/test", ({ params, body }): OracleTestAnswer => {
  const o = world.oracles.find((x) => x.oracle.workspace_id === params.ws && x.oracle.slug === params.slug);
  const draft = typeof body?.content === "string";
  if (!draft && !o) throw new DemoError(404, `no oracle ${params.slug} in this workspace`);
  const result = answer(o, draft);
  if (draft || !o) return { result, evaluation_id: null };
  const ev = { ...o.evaluations[0], id: `ev_demo_test_${clock.now()}`, created_at: clock.now(), params: (body?.params as Record<string, unknown>) ?? {}, callers: [{ caller: { kind: "test" as const, id: null, name: "Test now" }, at: clock.now(), how: "started", consequence: null, detail: null }] };
  if (o.evaluations.length) o.evaluations.unshift(ev);
  changed(o);
  return { result, evaluation_id: ev.id ?? null };
});
route("POST", "/api/oracles/:ws/:slug/run", ({ params }) => of(params.ws, params.slug).evaluations[0] ?? (() => { throw new DemoError(400, "never evaluated in the demo"); })());
