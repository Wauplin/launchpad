// Sessions, their events and the agent actions.

import type { SessionEventPage } from "@/gen/SessionEventPage";
import { DemoError, route } from "../daemon";
import { light, session, updateSession } from "../store";
import { world } from "../world";

const found = (id: string) => {
  const s = session(id);
  if (!s) throw new DemoError(404, "no such session");
  return s;
};

route("GET", "/api/sessions", (r) => world.sessions.filter((s) => r.query.get("include_assistant") === "true" || s.launch_spec.target.kind !== "assistant").map(light));
route("GET", "/api/sessions/:id", (r) => found(r.params.id));
route("GET", "/api/sessions/:id/events", (r) => (world.events[r.params.id] ?? []).filter((e) => e.seq > Number(r.query.get("after") ?? 0)));
route("GET", "/api/sessions/:id/events/page", (r): SessionEventPage => {
  const log = world.events[r.params.id] ?? [];
  const before = r.query.get("before");
  const end = before ? log.findIndex((e) => e.seq >= Number(before)) : log.length;
  const stop = end < 0 ? log.length : end;
  const size = Number(r.query.get("tail") ?? r.query.get("limit") ?? 200);
  return { events: log.slice(Math.max(0, stop - size), stop), facts: null };
});
route("POST", "/api/sessions/:id/seen", (r) => { const s = found(r.params.id); updateSession(s.id, { seen_at: Math.max(s.status_since, Date.now()) }); return { ok: true }; });
