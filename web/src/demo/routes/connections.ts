// Demo routes: Settings → Connections and the Outbox. Sending and hooks are simulated: nothing leaves the
// browser.

import type { Connection } from "@/gen/Connection";
import type { CreatedHook } from "@/gen/CreatedHook";
import type { OutboxMessage } from "@/gen/OutboxMessage";
import { toast } from "@/components/Toast";
import { clock, DemoError, publish, route } from "../daemon";
import { world } from "../world";

const ORIGIN = "https://demo-user-launchpad.hf.space";
const changed = () => publish("connections", "changed", null);
const messageOf = (id: string) => {
  const m = world.outbox.find((x) => x.id === id);
  if (!m) throw new DemoError(404, "no such Outbox message");
  return m;
};
const saved = (m: OutboxMessage) => { publish("outbox", "changed", m); return m; };

route("GET", "/api/connections", () => ({
  here: true, origin: ORIGIN, names: world.connections.map((c) => c.name),
  connections: world.connections, hooks: world.hooks,
}));
route("POST", "/api/connections", ({ body }) => {
  const name = String(body?.name ?? "").trim();
  if (!/^[a-z0-9][a-z0-9-]{0,39}$/.test(name)) throw new DemoError(400, "a connection name uses lowercase letters, digits and dashes (at most 40)");
  if (!String(body?.url ?? "").startsWith("https://")) throw new DemoError(400, "the webhook URL must start with https://");
  const c: Connection = { id: `demo-ch-${clock.now().toString(36)}`, name, kind: body.kind, host: new URL(body.url).host, created_at: clock.now() };
  world.connections = [...world.connections, c];
  changed();
  return c;
});
route("DELETE", "/api/connections/hooks/:id", ({ params }) => { world.hooks = world.hooks.filter((h) => h.id !== params.id); changed(); return { ok: true }; });
route("DELETE", "/api/connections/:id", ({ params }) => { world.connections = world.connections.filter((c) => c.id !== params.id); changed(); return { ok: true }; });
route("POST", "/api/connections/hooks", ({ body }): CreatedHook => {
  if (!String(body?.name ?? "").trim()) throw new DemoError(400, "give the hook a short name");
  const hook = { id: `demohook${clock.now().toString(36)}`, name: String(body.name).trim(), target: body.target, created_at: clock.now(), last_at: null, count: 0, last_session: null };
  world.hooks = [...world.hooks, hook];
  changed();
  return { hook, secret: "lph_demo_secret_not_real" };
});

route("GET", "/api/outbox", () => world.outbox);
route("PATCH", "/api/outbox/:id", ({ params, body }) => {
  const m = messageOf(params.id);
  if (m.status !== "draft") throw new DemoError(400, "only drafts can be edited");
  if (!String(body?.body ?? "").trim()) throw new DemoError(400, "the message is empty");
  m.body = body.body;
  m.title = body.title ?? null;
  return saved(m);
});
route("POST", "/api/outbox/:id/send", ({ params }) => {
  const m = messageOf(params.id);
  if (m.status !== "draft") throw new DemoError(400, "a send was already attempted; check the destination before discarding");
  m.status = "sent";
  m.sent_at = clock.now();
  toast(`Demo: marked as sent. Nothing was posted to ${m.connection}.`);
  return saved(m);
});
route("POST", "/api/outbox/:id/discard", ({ params }) => { const m = messageOf(params.id); m.status = "discarded"; return saved(m); });
route("POST", "/api/outbox/:id/mark-sent", ({ params }) => { const m = messageOf(params.id); m.status = "sent"; m.sent_at = clock.now(); return saved(m); });
