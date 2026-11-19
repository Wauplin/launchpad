// Pinned chats (`chat.pin`), kept in memory like the rest of the demo.
import { route, publish, DemoError } from "../daemon";

type Pin = { target: string; workspace_id: string; at: number };
const pins: Pin[] = [];

route("GET", "/api/chat-pins", ({ query }) => pins.filter((p) => p.workspace_id === query.get("workspace")));
route("POST", "/api/chat-pins", ({ body }) => {
  const target = String(body.target ?? "");
  if (!/^(conversation:[\w-]+:[\w-]+|channel:[\w-]+)$/.test(target)) throw new DemoError(400, "a chat pin names `conversation:<bot>:<thread>` or `channel:<id>`");
  const at = pins.findIndex((p) => p.target === target);
  if (at >= 0) pins.splice(at, 1);
  if (body.pinned) pins.push({ target, workspace_id: String(body.workspace_id), at: Date.now() });
  publish("chats", "pins", { workspace: body.workspace_id });
  return { ok: true, pinned: !!body.pinned };
});
