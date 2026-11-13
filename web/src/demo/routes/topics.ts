import { route, publish, DemoError } from "../daemon";
import { world } from "../world";
import type { Topic } from "@/gen/Topic";
const changed = () => publish("topics", "changed", {});
const find = (id: string) => { const t = world.topics.find((t) => t.id === id); if (!t) throw new DemoError(404, "Topic no longer exists"); return t; };
const validName = (name: string, workspace: string, id?: string) => {
  const n = name?.trim();
  if (!n || n.length > 80 || /[\r\n\t]/.test(n)) throw new DemoError(400, "Use a topic name of 1–80 characters");
  if (world.topics.some((t) => t.id !== id && t.workspace_id === workspace && t.name.toLowerCase() === n.toLowerCase())) throw new DemoError(400, "A topic with this name already exists");
  return n;
};
route("GET", "/api/topics", ({ query }) => world.topics.filter((t) => t.workspace_id === query.get("workspace")));
route("GET", "/api/topics/:id", ({ params }) => find(params.id));
route("POST", "/api/topics", ({ body }) => { const t: Topic = { id: crypto.randomUUID(), workspace_id: body.workspace_id, name: validName(body.name, body.workspace_id), session_ids: [] }; world.topics.push(t); changed(); return t; });
route("PATCH", "/api/topics/:id", ({ params, body }) => { const t = find(params.id); t.name = validName(body.name, t.workspace_id, t.id); changed(); return t; });
route("DELETE", "/api/topics/:id", ({ params }) => { find(params.id); world.topics = world.topics.filter((t) => t.id !== params.id); changed(); return { ok: true }; });
route("POST", "/api/topics/:id/members", ({ params, body }) => { const t = find(params.id); const ids: string[] = body.session_ids; if (ids.some((id) => !world.sessions.some((s) => s.id === id && (s.launch_spec.workspace_id === t.workspace_id || world.projects.some((p) => p.id === s.project_id && p.workspace_id === t.workspace_id))))) throw new DemoError(400, "Session belongs to another workspace"); t.session_ids = body.present ? [...new Set([...t.session_ids, ...ids])] : t.session_ids.filter((id) => !ids.includes(id)); changed(); return { ok: true }; });

route("GET", "/api/context/topics", ({ query }) => world.topics.filter(t => t.workspace_id === query.get("workspace")));

export function topicContext(receiver: string, selected: import("@/gen/TopicContext").TopicContext[] = [], direct: import("@/gen/AgentTraceRef").AgentTraceRef[] = []) {
  const refs = [...direct];
  const inherited = new Set(selected.map(t => t.id));
  for (const topic of world.topics) if (direct.some(r => topic.session_ids.includes(r.session_id))) inherited.add(topic.id);
  for (const t of selected) {
    const topic = find(t.id);
    t.snapshot ??= topic.session_ids.filter(id => id !== receiver).flatMap(id => { const s = world.sessions.find(s => s.id === id); return s && s.last_seq > 0 ? [{session_id:id,through_seq:s.last_seq}] : []; });
    refs.push(...t.snapshot);
  }
  for (const topic of world.topics) if (inherited.has(topic.id) && !topic.session_ids.includes(receiver)) topic.session_ids.push(receiver);
  if (inherited.size) changed();
  return refs.filter((r,i) => r.session_id !== receiver && refs.findIndex(v => v.session_id === r.session_id) === i);
}
