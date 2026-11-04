// Demo routes: the knowledge base of each workspace (Knowledge page). Edits, seen markers and concurrent edits
// change the demo world; restore, remove and import answer like the daemon without keeping history.

import type { Hit } from "@/gen/Hit";
import type { Notes } from "@/gen/Notes";
import { DemoError, publish, route } from "../daemon";
import { world } from "../world";
import { demoSha } from "../world/knowledge";

const GARDENER = "Tend the knowledge base of this workspace (its folder is $LAUNCHPAD_KB; read AGENTS.md there first). Merge duplicate notes, add missing [[wikilinks]], mark outdated notes `status: superseded`, merge every `*.concurrent-*` file into its note and remove it, and keep INDEX.md current.";

const kb = (ws: string) => {
  const k = world.knowledge[ws];
  if (!k) throw new DemoError(400, "no such workspace");
  return k;
};
const concurrentNote = (path: string) => /\.concurrent-[0-9a-f]{8}/.test(path) ? path.replace(/\.concurrent-[0-9a-f]{8}/, "") : null;
const changed = (ws: string) => publish("kb", "changed", { workspace_id: ws });

route("GET", "/api/knowledge/unseen", () => Object.fromEntries(Object.entries(world.knowledge).map(([ws, k]) => [ws, k.changes.filter((c) => c.id > k.seen && c.author?.kind === "agent").length])));
route("GET", "/api/knowledge/memories", () => ({ memories: world.memories }));
route("GET", "/api/knowledge/:ws", ({ params }): Notes => {
  const k = kb(params.ws);
  return {
    workspace_id: params.ws, version: k.next, seen: k.seen, gardener_prompt: GARDENER,
    files: k.notes.map((x) => ({ path: x.path, sha256: x.sha256, size: x.content.length, executable: false })).sort((a, b) => a.path.localeCompare(b.path)),
    concurrent: k.notes.flatMap((x) => { const note = concurrentNote(x.path); return note ? [{ path: x.path, note }] : []; }),
  };
});
route("GET", "/api/knowledge/:ws/file", ({ params, query }) => {
  const x = kb(params.ws).notes.find((n) => n.path === query.get("path"));
  if (!x) throw new DemoError(404, "no such note");
  return new Response(x.content, { headers: { "Content-Type": "text/plain; charset=utf-8" } });
});
route("PUT", "/api/knowledge/:ws/file", ({ params, body }) => {
  const k = kb(params.ws);
  const b = body as { path: string; content: string };
  const old = k.notes.find((n) => n.path === b.path);
  const sha256 = demoSha();
  if (old) Object.assign(old, { content: b.content, sha256 }); else k.notes.push({ path: b.path, content: b.content, sha256 });
  k.changes.unshift({ id: k.next++, path: b.path, sha256: old ? demoSha() : null, new_sha256: sha256, at: Date.now(), session_id: null, author: { kind: "owner", name: "You", session_id: null, model: null, runner: null }, added: b.content.split("\n").length, removed: 0, patch: b.content.split("\n").map((l) => `+${l}`).join("\n") });
  changed(params.ws);
  return { lines: [`Wrote ${b.path}`] };
});
route("DELETE", "/api/knowledge/:ws/file", ({ params, query }) => {
  const k = kb(params.ws);
  k.notes = k.notes.filter((n) => n.path !== query.get("path"));
  changed(params.ws);
  return { lines: [`Removed ${query.get("path")} (restorable for 30 days)`] };
});
route("GET", "/api/knowledge/:ws/changes", ({ params }) => ({ changes: kb(params.ws).changes.map(({ patch: _patch, ...c }) => c) }));
route("GET", "/api/knowledge/:ws/changes/:id/diff", ({ params }) => ({ patch: kb(params.ws).changes.find((c) => c.id === Number(params.id))?.patch ?? "" }));
route("GET", "/api/knowledge/:ws/search", ({ params, query }) => {
  const words = (query.get("q") ?? "").toLowerCase().split(/\s+/).filter(Boolean);
  const hits: Hit[] = [];
  for (const x of kb(params.ws).notes) x.content.split("\n").forEach((line, i) => { if (words.length && words.every((w) => line.toLowerCase().includes(w))) hits.push({ path: x.path, line: i + 1, text: line }); });
  return { hits };
});
route("POST", "/api/knowledge/:ws/seen", ({ params, body }) => {
  kb(params.ws).seen = (body as { change: number }).change;
  changed(params.ws);
  return { ok: true };
});
route("POST", "/api/knowledge/:ws/resolve", ({ params, body }) => {
  const k = kb(params.ws);
  const { path, use_other } = body as { path: string; use_other: boolean };
  const other = k.notes.find((n) => n.path === path);
  const note = concurrentNote(path);
  if (use_other && other && note) { const target = k.notes.find((n) => n.path === note); if (target) Object.assign(target, { content: other.content, sha256: demoSha() }); }
  k.notes = k.notes.filter((n) => n.path !== path);
  changed(params.ws);
  return { lines: [use_other ? `Wrote ${note}` : "Kept the note", `Removed ${path} (restorable for 30 days)`] };
});
route("POST", "/api/knowledge/:ws/restore", () => ({ lines: ["Restored (the demo keeps no history)"] }));
route("POST", "/api/knowledge/:ws/import", ({ params, body }) => {
  const k = kb(params.ws);
  const files = (body as { files: { path: string; content: string }[] }).files;
  for (const f of files) if (!k.notes.some((n) => n.path === f.path)) k.notes.push({ path: f.path, content: f.content, sha256: demoSha() });
  changed(params.ws);
  return { lines: [`Imported ${files.length} files`] };
});
