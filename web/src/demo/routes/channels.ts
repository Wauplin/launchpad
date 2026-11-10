// Demo routes: Settings → Channels and the Outbox. Sending, Telegram and hooks are simulated: nothing leaves the
// browser.

import type { Channel } from "@/gen/Channel";
import type { CreatedHook } from "@/gen/CreatedHook";
import type { OutboxMessage } from "@/gen/OutboxMessage";
import type { TelegramStatus } from "@/gen/TelegramStatus";
import { toast } from "@/components/Toast";
import { clock, DemoError, publish, route } from "../daemon";
import { world } from "../world";

const ORIGIN = "https://demo-user-launchpad.hf.space";
const changed = () => publish("channels", "changed", null);
const messageOf = (id: string) => {
  const m = world.outbox.find((x) => x.id === id);
  if (!m) throw new DemoError(404, "no such Outbox message");
  return m;
};
const saved = (m: OutboxMessage) => { publish("outbox", "changed", m); return m; };

route("GET", "/api/channels", () => ({
  here: true, origin: ORIGIN, names: [...(bound() ? ["telegram"] : []), ...world.channels.map((c) => c.name)],
  channels: world.channels, hooks: world.hooks,
}));
route("POST", "/api/channels", ({ body }) => {
  const name = String(body?.name ?? "").trim();
  if (!/^[a-z0-9][a-z0-9-]{0,39}$/.test(name) || name === "telegram") throw new DemoError(400, "a channel name uses lowercase letters, digits and dashes (at most 40)");
  if (!String(body?.url ?? "").startsWith("https://")) throw new DemoError(400, "the webhook URL must start with https://");
  const c: Channel = { id: `demo-ch-${clock.now().toString(36)}`, name, kind: body.kind, host: new URL(body.url).host, created_at: clock.now() };
  world.channels = [...world.channels, c];
  changed();
  return c;
});
route("DELETE", "/api/channels/hooks/:id", ({ params }) => { world.hooks = world.hooks.filter((h) => h.id !== params.id); changed(); return { ok: true }; });
// Each workspace's own Telegram bot (Settings → Workspace).
const NONE = (ws: string): TelegramStatus => ({ workspace_id: ws, configured: false, bot_username: null, bound: false, chat_name: null, code: null, link: null, code_expires_at: null, notify: true, titles: false, error: null });
const telegramOf = (ws: string) => {
  if (!world.workspaces.some((w) => w.id === ws)) throw new DemoError(400, "no such workspace");
  return world.telegram[ws] ?? NONE(ws);
};
const setTelegram = (ws: string, t: TelegramStatus) => { world.telegram = { ...world.telegram, [ws]: t }; changed(); return t; };
const bound = () => Object.values(world.telegram).some((t) => t.bound);
route("GET", "/api/channels/telegram/:ws", ({ params }) => telegramOf(params.ws));
route("DELETE", "/api/channels/telegram/:ws", ({ params }) => { telegramOf(params.ws); world.telegram = Object.fromEntries(Object.entries(world.telegram).filter(([k]) => k !== params.ws)); changed(); return { ok: true }; });
route("PUT", "/api/channels/telegram/:ws/token", ({ params }) => {
  const name = `${world.workspaces.find((w) => w.id === params.ws)?.name.toLowerCase().replace(/[^a-z0-9]/g, "") ?? "demo"}_launchpad_bot`;
  if (Object.values(world.telegram).some((t) => t.workspace_id !== params.ws && t.bot_username === name)) throw new DemoError(400, "this Telegram bot already serves another workspace");
  return setTelegram(params.ws, { ...telegramOf(params.ws), configured: true, bound: false, bot_username: name, code: "DEMO7K2Q", link: `https://t.me/${name}?start=DEMO7K2Q`, code_expires_at: clock.now() + 30 * 60_000 });
});
route("POST", "/api/channels/telegram/:ws/code", ({ params }) => {
  const t = telegramOf(params.ws);
  if (!t.configured) throw new DemoError(400, "add the bot token first");
  return setTelegram(params.ws, { ...t, code: "DEMO9X4M", link: `https://t.me/${t.bot_username}?start=DEMO9X4M`, code_expires_at: clock.now() + 30 * 60_000 });
});
route("PATCH", "/api/channels/telegram/:ws", ({ params, body }) => {
  const t = telegramOf(params.ws);
  return setTelegram(params.ws, { ...t, ...(typeof body?.notify === "boolean" ? { notify: body.notify } : {}), ...(typeof body?.titles === "boolean" ? { titles: body.titles } : {}) });
});
route("POST", "/api/channels/telegram/:ws/test", () => { toast("Demo: no Telegram message was sent."); return { ok: true }; });
route("DELETE", "/api/channels/:id", ({ params }) => { world.channels = world.channels.filter((c) => c.id !== params.id); changed(); return { ok: true }; });
route("POST", "/api/channels/hooks", ({ body }): CreatedHook => {
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
  toast(`Demo: marked as sent. Nothing was posted to ${m.channel}.`);
  return saved(m);
});
route("POST", "/api/outbox/:id/discard", ({ params }) => { const m = messageOf(params.id); m.status = "discarded"; return saved(m); });
route("POST", "/api/outbox/:id/mark-sent", ({ params }) => { const m = messageOf(params.id); m.status = "sent"; m.sent_at = clock.now(); return saved(m); });
