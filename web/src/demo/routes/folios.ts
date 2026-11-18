// Demo routes: folios (Home's `crates/api/src/folios.rs`), answered from the demo world. Pages are split with the
// UI's splitter (the same rules as Home's); marks, replies, Archive and Send feedback change the demo world. Nothing
// checks pages here: the demo folio has no problems.

import type { Folio } from "@/gen/Folio";
import type { FolioDetail } from "@/gen/FolioDetail";
import type { FolioMark } from "@/gen/FolioMark";
import type { FolioTarget } from "@/gen/FolioTarget";
import { outline } from "@/lib/folioSplit";
import { DemoError, clock, publish, route } from "../daemon";
import { world } from "../world";
import type { DemoFolio } from "../world/folios";

const of = (ws: string, slug: string): DemoFolio => {
  const f = world.folios.find((x) => x.folio.workspace_id === ws && (x.folio.slug === slug || x.folio.id === slug));
  if (!f) throw new DemoError(404, `no folio ${slug} in this workspace`);
  return f;
};
const owner = (m: FolioMark) => m.author.kind === "owner" && m.kind !== "reply";
const fill = (f: DemoFolio): Folio => ({ ...f.folio, unsent: f.marks.filter((m) => owner(m) && m.sent_at == null).length, open: f.marks.filter((m) => m.kind === "comment" && m.resolved_at == null).length });
const changed = (f: DemoFolio) => publish("folios", "changed", { workspace_id: f.folio.workspace_id, id: f.folio.id, slug: f.folio.slug });
const seq = () => Math.max(0, ...world.folios.flatMap((f) => f.marks.map((m) => m.seq))) + 1;
const PAGES = ["index.md", "plan.md"];

route("GET", "/api/folios/:ws", ({ params, query }) => world.folios.filter((f) => f.folio.workspace_id === params.ws && (f.folio.state === "archived") === (query.get("archived") === "true")).map(fill));
route("GET", "/api/folios/:ws/_activity", ({ params }) => world.folios.filter((f) => f.folio.workspace_id === params.ws).flatMap((f) => f.revs.map((r) => ({ folio_id: f.folio.id, slug: f.folio.slug, title: f.folio.title, rev: r.rev, at: r.at, session_id: r.session_id, paths: r.files.map((x) => x.path), created: r.rev === 1 }))));
route("GET", "/api/folios/:ws/_session/:session", ({ params }) => world.folios.filter((f) => f.folio.workspace_id === params.ws && f.folio.links.some((l) => l.session_id === params.session)).map(fill));
route("GET", "/api/folios/:ws/:slug", ({ params }): FolioDetail => {
  const f = of(params.ws, params.slug);
  const pages = PAGES.filter((p) => f.files[p] != null).map((path) => {
    const text = f.files[path];
    const o = outline(text);
    const title = /^title:\s*(.+)$/m.exec(text)?.[1]?.trim() ?? o.items.find((i) => i.t === "heading")?.text ?? path;
    return { path, title: path === "index.md" ? f.folio.title : title, text, outline: o };
  });
  return { folio: fill(f), rev: f.folio.rev, pages, files: Object.keys(f.files).map((path) => ({ path, sha256: `demo-${path}`, size: f.files[path].length, executable: false })), problems: [], marks: f.marks };
});
route("GET", "/api/folios/:ws/:slug/file", ({ params, query }) => {
  const text = of(params.ws, params.slug).files[query.get("path") ?? ""];
  if (text == null) throw new DemoError(404, "there is no such file in this folio");
  return new Response(text, { headers: { "Content-Type": "text/plain; charset=utf-8" } });
});
route("GET", "/api/folios/:ws/:slug/revs", ({ params }) => of(params.ws, params.slug).revs);
route("GET", "/api/folios/:ws/:slug/revs/:rev", ({ params }) => {
  const r = of(params.ws, params.slug).revs.find((x) => String(x.rev) === params.rev);
  return { rev: Number(params.rev), files: (r?.files ?? []).map((x) => ({ path: x.path, patch: x.before ? "@@ -12,3 +12,4 @@\n :::callout{tone=verdict} Verdict\n-Ship the store and reader first.\n+Ship the store and reader first. Comments come second, because they need stable anchors.\n+Sharing comes last.\n :::" : "(new file)", added: x.added, removed: x.removed, created: x.before == null, deleted: x.after == null })) };
});
route("POST", "/api/folios/:ws/:slug/revs/:rev/restore", ({ params }) => { const f = of(params.ws, params.slug); changed(f); return { folio: fill(f), written: false, rev: f.folio.rev, lines: ["Demo: nothing to restore"], problems: [] }; });
route("GET", "/api/folios/:ws/:slug/check", ({ params }) => ({ slug: params.slug, rev: of(params.ws, params.slug).folio.rev, problems: [] }));
route("GET", "/api/folios/:ws/:slug/marks", ({ params }) => of(params.ws, params.slug).marks);
route("POST", "/api/folios/:ws/:slug/marks", ({ params, body }) => {
  const f = of(params.ws, params.slug);
  const b = body as Partial<FolioMark> & { kind: string; id?: number };
  const value = typeof b.value === "string" && b.value ? b.value : null;
  const now = clock.now();
  if (b.kind !== "comment") {
    const old = f.marks.find((m) => m.kind === b.kind && m.target === b.target && m.author.kind === "owner");
    if (!value) { f.marks = f.marks.filter((m) => m !== old); changed(f); return null; }
    if (old) { Object.assign(old, { value, sent_at: null, updated_at: now, seq: seq() }); changed(f); return old; }
  }
  const m: FolioMark = { id: Math.max(0, ...f.marks.map((x) => x.id)) + 1, seq: seq(), kind: b.kind, author: { kind: "owner", name: "You", session_id: null, model: null, runner: null }, page: b.page ?? "index.md", target: b.target ?? null, quote: b.quote ?? null, prefix: b.prefix ?? null, suffix: b.suffix ?? null, value, text: b.text ?? null, parent: null, rev: f.folio.rev, created_at: now, updated_at: now, sent_at: null, resolved_at: null, resolved_rev: null };
  f.marks.push(m);
  changed(f);
  return m;
});
route("DELETE", "/api/folios/:ws/:slug/marks/:id", ({ params }) => { const f = of(params.ws, params.slug); f.marks = f.marks.filter((m) => String(m.id) !== params.id); changed(f); return { ok: true }; });
route("POST", "/api/folios/:ws/:slug/marks/:id/resolve", ({ params, body }) => {
  const f = of(params.ws, params.slug);
  const m = f.marks.find((x) => String(x.id) === params.id);
  if (!m) throw new DemoError(404, "no such comment");
  const open = (body as { open?: boolean } | null)?.open === true;
  Object.assign(m, { resolved_at: open ? null : clock.now(), resolved_rev: open ? null : f.folio.rev, seq: seq() });
  changed(f);
  return m;
});
route("POST", "/api/folios/:ws/:slug/archive", ({ params, body }) => {
  const f = of(params.ws, params.slug);
  const archived = (body as { archived?: boolean } | null)?.archived !== false;
  f.folio = { ...f.folio, state: archived ? "archived" : "active", archived_at: archived ? clock.now() : null };
  changed(f);
  return fill(f);
});
route("DELETE", "/api/folios/:ws/:slug", ({ params }) => { const f = of(params.ws, params.slug); world.folios = world.folios.filter((x) => x !== f); changed(f); return { ok: true }; });
route("GET", "/api/folios/:ws/:slug/targets", ({ params }): FolioTarget[] => {
  const f = of(params.ws, params.slug);
  const s = world.sessions.find((x) => x.id === f.folio.links[0]?.session_id);
  return s ? [{ kind: "session", bot_id: null, thread_id: null, session_id: s.id, label: `Session “${s.title}”`, detail: `created · rev ${f.folio.rev}` }] : [];
});
route("POST", "/api/folios/:ws/:slug/send", ({ params, body }) => {
  const f = of(params.ws, params.slug);
  const unsent = f.marks.filter((m) => owner(m) && m.sent_at == null);
  if (!unsent.length && !(body as { note?: string })?.note) throw new DemoError(400, "nothing to send: comment, decide or mark items first");
  for (const m of unsent) m.sent_at = clock.now();
  changed(f);
  const b = body as { kind: string; session_id?: string; bot_id?: string };
  return { session_id: b.session_id ?? f.folio.links[0]?.session_id ?? "", bot_id: b.kind === "session" ? null : b.bot_id ?? null, thread: null, sent: unsent.length };
});
route("POST", "/api/folios/:ws/:slug/ask-fix", ({ params, body }) => {
  const f = of(params.ws, params.slug);
  const b = body as { kind: string; session_id?: string; bot_id?: string };
  return { session_id: b.session_id ?? f.folio.links[0]?.session_id ?? "", bot_id: b.kind === "session" ? null : b.bot_id ?? null, thread: null };
});
